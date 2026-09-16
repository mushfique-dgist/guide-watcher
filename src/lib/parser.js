/**
 * Parse a line of Claude stream-json output into a typed step object.
 * Returns null for events we want to silently ignore.
 *
 * Claude's actual stream-json format emits turns as:
 *   {"type":"system","subtype":"init",...}
 *   {"type":"assistant","message":{"content":[{type:"tool_use",name,input},{type:"text",text}],...}}
 *   {"type":"user","message":{"content":[{type:"tool_result",...}],...}}
 *   {"type":"result",...}
 */
export function parseLine(raw) {
  return parseLineEvents(raw)[0] ?? null;
}

/**
 * Parse every meaningful event carried by one CLI line. Claude can place
 * multiple content blocks in one assistant event, so callers that display a
 * stream should use this function instead of silently dropping later blocks.
 */
export function parseLineEvents(raw) {
  // Stderr lines from CLI runners.
  if (raw.startsWith('__stderr__')) {
    const t = raw.slice(10).trim();
    if (!t || t.length < 2) return [];
    if (isOptionalMcpAuthNoise(t)) {
      return [{ type: 'system', text: 'Optional connector auth unavailable; continuing without it.' }];
    }
    if (/^debug:|^trace:/i.test(t)) return [];
    return [{ type: 'raw', text: t, isError: /error/i.test(t) }];
  }

  // Phase separator emitted by the Rust backend between two-phase job stages
  if (raw.startsWith('__phase__')) {
    const text = raw.slice(9).trim();
    return [{ type: 'phase', text, activity: text }];
  }

  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    const t = raw.trim();
    if (!t || t.length < 3) return [];
    return [{ type: 'raw', text: t, isError: /error/i.test(t) }];
  }
  const parsed = parseEvent(data);
  if (parsed !== undefined) {
    if (Array.isArray(parsed)) return parsed.filter(Boolean);
    return parsed ? [parsed] : [];
  }
  return [{
    type: 'raw',
    text: summarizeUnknownEvent(data),
    isError: /error|failed/i.test(JSON.stringify(data)),
  }];
}

function parseEvent(e) {
  const codex = parseCodexEvent(e);
  if (codex !== undefined) return codex;
  return parseClaudeEvent(e);
}

function parseClaudeEvent(e) {
  const t = e.type || '', s = e.subtype || '';

  // ── System ──────────────────────────────────────────────
  if (t === 'init') return null;
  if (t === 'system') return { type: 'system', text: 'Initializing…' };

  // ── Completion (includes full usage summary) ─────────────
  if (t === 'result') {
    const u = e.usage || {};
    return {
      type: 'complete',
      inputTokens:  u.input_tokens  || 0,
      outputTokens: u.output_tokens || 0,
      cacheRead:    u.cache_read_input_tokens || 0,
      cacheWrite:   u.cache_creation_input_tokens || 0,
      costUsd:      e.total_cost_usd || null,
      durationMs:   e.duration_ms   || null,
      numTurns:     e.num_turns      || null,
    };
  }

  // ── Assistant turn (main event type in stream-json) ──────
  // Content is an array of tool_use and/or text blocks.
  if (t === 'assistant') {
    const content = e.message?.content;
    if (!Array.isArray(content)) return null;
    // Per-turn token delta for live counter
    const u = e.message?.usage;
    const tokenDelta = u ? { inputTokens: u.input_tokens || 0, outputTokens: u.output_tokens || 0 } : null;

    const steps = [];
    for (const block of content) {
      if (block.type === 'tool_use') {
        const detail = extractDetail(block.input || {});
        steps.push({
          type: 'tool_start',
          tool: block.name,
          detail,
          tag: toolTag(block.name),
          activity: detail ? `${block.name}: ${detail}` : `${block.name}…`,
        });
      } else if (block.type === 'text' && block.text?.trim()) {
        steps.push({ type: 'text_chunk', text: block.text });
      }
    }
    // Preserve every block in order. Attach the per-turn token delta once so
    // aggregate counters do not double count multi-block assistant messages.
    if (steps.length > 0 && tokenDelta) steps[0] = { ...steps[0], tokenDelta };
    if (steps.length > 0) return steps;
    if (tokenDelta) return [{ type: 'token_delta', ...tokenDelta }];
    return [];
  }

  // ── User turn = tool results (signals completion) ─────────
  if (t === 'user') {
    const content = e.message?.content;
    if (!Array.isArray(content)) return null;
    const hasResult = content.some(b => b.type === 'tool_result');
    return hasResult ? { type: 'tool_done', tool: '' } : null;
  }

  // ── Fallback: some verbose modes emit individual tool events ──
  if (s === 'tool_use' || t === 'tool_use') {
    const tool = e.tool_name || e.name || 'Tool';
    const detail = extractDetail(e.input || {});
    return {
      type: 'tool_start',
      tool,
      detail,
      tag: toolTag(tool),
      activity: detail ? `${tool}: ${detail}` : `${tool}…`,
    };
  }

  if (s === 'tool_result' || t === 'tool_result') {
    return { type: 'tool_done', tool: e.tool_name || e.name || '' };
  }

  if (s === 'text' || t === 'text') {
    const text = e.text || '';
    return text.trim() ? { type: 'text_chunk', text } : null;
  }

  return undefined;
}

function parseCodexEvent(e) {
  const type = e.type || e.event || '';
  const item = e.item || e.data?.item || e.event_msg || e.message || {};
  const itemType = item.type || item.kind || e.item_type || '';

  if (type === 'thread.started') {
    return { type: 'system', text: 'Codex session started' };
  }
  if (type === 'turn.started') {
    return { type: 'system', text: 'Model turn started' };
  }
  if (type === 'turn.completed') {
    return { type: 'system', text: 'Model turn complete' };
  }
  if (type === 'error' || type === 'turn.failed') {
    return {
      type: 'raw',
      text: cleanCodexText(e.error?.message || e.message || e.msg || 'Codex run failed'),
      isError: true,
    };
  }

  if (type === 'item.started') {
    if (isReasoningItem(itemType)) {
      return { type: 'tool_start', tool: 'Thinking', detail: 'Reasoning through the task', tag: 'agent', activity: 'Thinking…' };
    }
    if (isMessageItem(itemType)) {
      return { type: 'system', text: 'Writing response' };
    }
    if (isToolCallItem(itemType)) {
      const tool = codexToolName(item);
      const detail = codexToolDetail(item);
      return {
        type: 'tool_start',
        tool,
        detail,
        tag: toolTag(tool),
        activity: detail ? `${tool}: ${detail}` : `${tool}…`,
      };
    }
    return null;
  }

  if (type === 'item.completed') {
    if (isReasoningItem(itemType)) {
      const text = codexText(item);
      if (text) return { type: 'text_chunk', text: `[thinking]\n${text}\n` };
      return { type: 'tool_done', tool: 'Thinking' };
    }
    if (isMessageItem(itemType)) {
      const text = codexText(item);
      return text ? { type: 'text_chunk', text } : null;
    }
    if (isToolCallItem(itemType)) {
      return { type: 'tool_done', tool: codexToolName(item) };
    }
    const output = codexOutput(item);
    if (output) {
      return { type: 'text_chunk', text: output };
    }
    return null;
  }

  if (type === 'response.output_text.delta' || type === 'text.delta') {
    const text = e.delta || e.text || '';
    return text ? { type: 'text_chunk', text } : null;
  }

  if (type === 'response.completed' || type === 'completed') {
    const usage = e.response?.usage || e.usage || {};
    return {
      type: 'complete',
      inputTokens: usage.input_tokens || usage.inputTokens || 0,
      outputTokens: usage.output_tokens || usage.outputTokens || 0,
      cacheRead: usage.input_token_details?.cached_tokens || usage.cache_read_input_tokens || 0,
      cacheWrite: usage.cache_creation_input_tokens || 0,
      costUsd: null,
      durationMs: e.duration_ms || null,
      numTurns: null,
    };
  }

  // Codex emits many structural events. If no content/tool detail exists,
  // silence them instead of showing item.started/item.completed noise.
  if (/^(thread|turn|item)\./.test(type)) return null;

  return undefined;
}

function summarizeUnknownEvent(e) {
  const type = e.type || e.event || e.kind || 'event';
  const msg =
    e.message?.content?.[0]?.text
    || e.message
    || e.text
    || e.msg
    || e.delta
    || '';
  const text = typeof msg === 'string' ? msg : JSON.stringify(msg);
  const summary = text ? `${type}: ${text}` : type;
  return summary.length > 180 ? summary.slice(0, 177) + '...' : summary;
}

function isOptionalMcpAuthNoise(text) {
  return /AuthRequired|invalid_token|Missing or invalid access token|well-known\/oauth|oauth-protected-resource/i.test(text);
}

function isReasoningItem(type) {
  return ['reasoning', 'thinking', 'thought'].includes(String(type).toLowerCase());
}

function isMessageItem(type) {
  return ['message', 'assistant_message', 'output_text'].includes(String(type).toLowerCase());
}

function isToolCallItem(type) {
  return ['function_call', 'tool_call', 'local_shell_call', 'command_execution', 'mcp_tool_call'].includes(String(type).toLowerCase());
}

function codexText(item) {
  const content = item.content || item.text || item.summary || item.output_text || '';
  if (typeof content === 'string') return cleanCodexText(content);
  if (Array.isArray(content)) {
    return cleanCodexText(content.map(part => {
      if (typeof part === 'string') return part;
      return part.text || part.content || part.output_text || '';
    }).filter(Boolean).join('\n'));
  }
  return '';
}

function codexOutput(item) {
  const output = item.output || item.result || item.call_output || '';
  if (typeof output === 'string') return cleanCodexText(output);
  return '';
}

function codexToolName(item) {
  return item.name || item.tool_name || item.server_label || item.call?.name || item.action || 'Tool';
}

function codexToolDetail(item) {
  const args = item.arguments || item.input || item.call?.arguments || {};
  if (typeof args === 'string') {
    return args.length > 90 ? args.slice(0, 87) + '...' : args;
  }
  return extractDetail(args);
}

function cleanCodexText(text) {
  const s = String(text || '').trim();
  return s.length > 3000 ? s.slice(0, 2997) + '...' : s;
}

function extractDetail(inp) {
  if (inp.file_path) return inp.file_path.replace(/\\/g, '/').split('/').pop();
  if (inp.command)   return inp.command.length > 70 ? inp.command.slice(0, 67) + '…' : inp.command;
  if (inp.pattern)   return inp.pattern;
  if (inp.glob)      return inp.glob;
  if (inp.description) return inp.description.length > 50 ? inp.description.slice(0, 47) + '…' : inp.description;
  return '';
}

function toolTag(tool) {
  if (['Read', 'Glob', 'Grep'].includes(tool)) return 'read';
  if (['Bash', 'shell', 'local_shell'].includes(tool)) return 'bash';
  if (['Write', 'Edit'].includes(tool)) return 'write';
  if (['Agent', 'Task'].includes(tool)) return 'agent';
  return 'other';
}
