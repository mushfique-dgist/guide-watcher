/**
 * Parse a line of Claude stream-json output.
 * @param {string} raw - raw JSON string from stdout
 * @returns {{ text: string, tag: string|null, activity: string|null }}
 */
export function parseLine(raw) {
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    // Non-JSON line
    const trimmed = raw.length > 150 ? raw.slice(0, 147) + '...' : raw;
    const isError = /error/i.test(raw);
    return {
      text: trimmed,
      tag: isError ? 'error' : 'dim',
      activity: null,
    };
  }

  return parseJsonEvent(data);
}

function parseJsonEvent(data) {
  const type = data.type || '';
  const subtype = data.subtype || '';

  // Tool use events
  if (subtype === 'tool_use' || type === 'tool_use') {
    const tool = data.tool_name || data.name || 'unknown';
    const input = data.input || {};

    const toolLabels = {
      Read: 'Read', Bash: 'Bash', Write: 'Write', Edit: 'Edit',
      Glob: 'Search', Grep: 'Grep', Agent: 'Sub-agent', TodoWrite: 'Planning',
    };
    const label = toolLabels[tool] || tool;

    // Extract detail from input
    let detail = '';
    if (input.file_path) {
      detail = input.file_path.replace(/\\/g, '/').split('/').pop();
    } else if (input.command) {
      detail = input.command.length > 55
        ? input.command.slice(0, 52) + '...'
        : input.command;
    } else if (input.pattern) {
      detail = input.pattern;
    }

    const text = detail ? `▸ ${label}: ${detail}` : `▸ ${label}...`;
    const tag = getToolTag(tool);

    return { text, tag, activity: detail ? `${label}: ${detail}` : `${label}...` };
  }

  // Tool results
  if (subtype === 'tool_result' || type === 'tool_result') {
    const tool = data.tool_name || data.name || '';
    return { text: tool ? `✓ ${tool} done` : '✓ done', tag: 'dim', activity: null };
  }

  // Text generation
  if (subtype === 'text' || type === 'text') {
    const text = (data.text || '').trim();
    if (text.length > 5) {
      const snippet = text.slice(0, 80);
      return { text: `▸ Writing: ${snippet}`, tag: 'text', activity: null };
    }
    return { text: '▸ Generating...', tag: 'text', activity: null };
  }

  // Result / completion
  if (type === 'result') {
    return { text: 'Generation complete', tag: 'header', activity: null };
  }

  // System / init
  if (type === 'system' || type === 'init') {
    return { text: 'Initializing...', tag: 'dim', activity: null };
  }

  // Fallback
  return { text: JSON.stringify(data).slice(0, 150), tag: 'dim', activity: null };
}

function getToolTag(tool) {
  if (['Read', 'Glob', 'Grep'].includes(tool)) return 'read';
  if (tool === 'Bash') return 'bash';
  if (['Write', 'Edit'].includes(tool)) return 'write';
  return 'read';
}
