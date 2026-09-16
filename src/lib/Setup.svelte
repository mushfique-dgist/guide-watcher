<script>
  // First run. A downloaded build knows nothing about this computer, so it asks — once — and
  // checks every answer against the machine before it calls itself ready.
  //
  // Two doors lead to the same settings file. One walks the person through the questions. The
  // other hands the work to an AI assistant that is already signed in here, and waits for it.
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { writeText } from '@tauri-apps/plugin-clipboard-manager';
  import { emptySubject, defaultFolderFor, normalizeSubjects, draftProblems, blockingRequirements, lectureFilesProblem } from './setupFlow.js';

  // `edit` is the same screen reached later from Settings, where there is nothing to announce as
  // ready: the person came to change an answer, not to be congratulated.
  let { onready = () => {}, oncancel = null, edit = false } = $props();

  let door = $state(edit ? 'manual' : 'choose');  // choose · manual · assisted
  let state_ = $state(null);             // the machine, as the backend sees it
  let error = $state('');
  let busy = $state(false);
  let saved = $state(false);
  let handoff = $state(null);
  let copied = $state(false);

  let watchDir = $state('');
  let automationDir = $state('');
  let subjects = $state([emptySubject()]);

  const problems = $derived(draftProblems({ watchDir, automationDir, subjects }));
  const blocking = $derived(blockingRequirements(state_));
  const ready = $derived(!!state_?.ready);

  async function refresh() {
    try { state_ = await invoke('setup_state'); error = ''; }
    catch (e) { error = String(e); }
  }

  async function load() {
    await refresh();
    // Answers already on this machine are the starting point; suggestions only fill the gaps.
    const current = state_?.settings ?? {};
    const [suggestedStudy, suggestedAutomation] = await invoke('setup_suggestions').catch(() => ['', '']);
    watchDir = current.watch_dir || suggestedStudy || '';
    automationDir = current.automation_dir || suggestedAutomation || '';
    subjects = (current.courses ?? []).length
      ? current.courses.map(course => ({ label: course.label, folder: course.folder, lectureFiles: course.lecture_files || '' }))
      : [emptySubject()];
  }

  async function choose(which, target) {
    error = '';
    try {
      const chosen = await invoke('pick_folder', { title: which });
      if (!chosen) return;
      if (target === 'watch') watchDir = chosen;
      else if (target === 'automation') automationDir = chosen;
      else subjects[target].folder = chosen;
    } catch (e) { error = String(e); }
  }

  function addSubject() { subjects = [...subjects, emptySubject()]; }
  function removeSubject(index) { subjects = subjects.filter((_, at) => at !== index); if (!subjects.length) subjects = [emptySubject()]; }
  function nameChanged(index) {
    // The folder follows the name until the person changes it themselves.
    const subject = subjects[index];
    if (!subject.folderTouched) subject.folder = defaultFolderFor(subject.label);
  }

  async function save() {
    error = ''; saved = false; busy = true;
    try {
      state_ = await invoke('save_setup', { watchDir, automationDir, subjects: normalizeSubjects(subjects) });
      saved = true;
    } catch (e) { error = String(e); }
    finally { busy = false; }
  }

  async function handToAssistant() {
    error = ''; busy = true; door = 'assisted';
    try { handoff = await invoke('start_setup_assistant'); }
    catch (e) { error = String(e); }
    finally { busy = false; }
  }

  async function copyCommand() {
    try { await writeText(handoff.command, { label: 'Guide Watcher setup command' }); copied = true; }
    catch { error = 'Could not copy the command. Select it and copy it by hand.'; }
  }

  onMount(() => {
    load();
    // While the assistant works, the app keeps asking the machine rather than the assistant.
    const timer = setInterval(() => { if (door === 'assisted' || saved) refresh(); }, 3000);
    return () => clearInterval(timer);
  });
</script>

<section class="setup" aria-labelledby="setup-title">
  <div class="setup-inner">
    <header>
      <h1 id="setup-title">{edit ? 'What this computer is pointed at' : 'Set up Guide Watcher'}</h1>
      <p>{edit ? 'Change where your course material lives, which subjects you keep, and which files start a guide. Everything is checked against this computer before it is saved.' : 'This app writes study guides from your own course material. It needs to know where that material lives, and which tools on this computer it may use. You only do this once.'}</p>
    </header>

    {#if error}<p class="error-box" role="alert">{error}</p>{/if}

    {#if ready && !edit}
      <div class="done" role="status">
        <h2>Everything is ready</h2>
        <p>Your settings are saved at <code>{state_.settingsPath}</code>. You can change any of this later in Settings.</p>
        <button class="button primary" onclick={onready}>Start using Guide Watcher</button>
      </div>
    {:else if door === 'choose'}
      <div class="doors">
        <button class="door" onclick={() => door = 'manual'}>
          <h2>Walk me through it</h2>
          <p>Three questions, each checked against this computer as you answer. Takes about two minutes.</p>
          <span class="door-go">Start →</span>
        </button>
        <button class="door" disabled={!state_?.assistantAvailable} onclick={handToAssistant}>
          <h2>Let my AI assistant do it</h2>
          <p>{state_?.assistantAvailable
            ? 'Opens a terminal with Claude Code, already signed in here, and hands it written instructions. It will ask you where your course material lives. Guide Watcher waits and continues by itself.'
            : 'Needs Claude Code installed and signed in on this computer.'}</p>
          <span class="door-go">{state_?.assistantAvailable ? 'Hand it over →' : (state_?.assistantDetail || 'Not available')}</span>
        </button>
      </div>
      <p class="muted">Either way the result is the same file, which you can read and edit yourself.</p>
    {:else if door === 'assisted'}
      <div class="waiting">
        <h2>Waiting for your assistant</h2>
        {#if busy}
          <p role="status">Opening a terminal…</p>
        {:else if handoff}
          <p role="status">{handoff.detail}</p>
          {#if !handoff.terminalOpened}
            <p>Open a terminal yourself and run this:</p>
            <div class="command-row">
              <code>{handoff.command}</code>
              <button class="button secondary" onclick={copyCommand}>{copied ? 'Copied' : 'Copy'}</button>
            </div>
          {/if}
          <p class="muted">Instructions written for it: <code>{handoff.briefPath}</code></p>
        {/if}
        <div class="pulse" aria-hidden="true"><span></span><span></span><span></span></div>
        <p class="muted">This screen checks the machine every few seconds. The moment your assistant writes the settings, Guide Watcher continues on its own.</p>
        {#if blocking.length}
          <ul class="checks">
            {#each blocking as item}<li><strong>{item.label}</strong><span>{item.detail}</span></li>{/each}
          </ul>
        {/if}
        <button class="button ghost" onclick={() => door = 'manual'}>I would rather do it myself</button>
      </div>
    {:else}
      <div class="questions">
        <section>
          <h2>1 · Where is your course material?</h2>
          <p class="muted">The folder that holds one folder per subject. Nothing outside it is ever read.</p>
          <div class="path-row">
            <input aria-label="Study folder" bind:value={watchDir} spellcheck="false" placeholder="for example C:/Users/you/Documents/Study"/>
            <button class="button secondary" onclick={() => choose('Choose your study folder', 'watch')}>Choose…</button>
          </div>
        </section>

        <section>
          <h2>2 · Where are the writing rules?</h2>
          <p class="muted">The <code>_automation</code> folder that came with Guide Watcher. It holds the writing rules, the depth contract and the checker.</p>
          <div class="path-row">
            <input aria-label="Automation folder" bind:value={automationDir} spellcheck="false" placeholder="the _automation folder"/>
            <button class="button secondary" onclick={() => choose('Choose the _automation folder', 'automation')}>Choose…</button>
          </div>
        </section>

        <section>
          <h2>3 · Which subjects?</h2>
          <p class="muted">A name you recognise, and the folder inside your study folder that holds its material.</p>
          {#each subjects as subject, index}
            <div class="subject">
              <input aria-label={`Subject ${index + 1} name`} bind:value={subject.label} oninput={() => nameChanged(index)} placeholder="Computer Networks"/>
              <input aria-label={`Subject ${index + 1} folder`} bind:value={subject.folder} oninput={() => subject.folderTouched = true} placeholder="folder name"/>
              <button class="button secondary" onclick={() => choose(`Choose the folder for ${subject.label || 'this subject'}`, index)}>Choose…</button>
              <button class="button ghost" aria-label={`Remove subject ${index + 1}`} onclick={() => removeSubject(index)}>×</button>
              <details class="advanced">
                <summary>Only some files start a guide</summary>
                <label for={`pattern-${index}`}>File-name pattern</label>
                <input id={`pattern-${index}`} bind:value={subject.lectureFiles} spellcheck="false" placeholder="for example ^chapter"/>
                <p class="muted">Leave this empty unless only certain files should start a guide. Everything else in the folder is still used as supporting material.</p>
                {#if lectureFilesProblem(subject.lectureFiles)}<p class="problem">{lectureFilesProblem(subject.lectureFiles)}</p>{/if}
              </details>
            </div>
          {/each}
          <button class="button ghost add-subject" onclick={addSubject}>+ Add another subject</button>
        </section>

        {#if saved && !problems.length}<p class="saved" role="status">Saved. These settings apply to the next guide you start.</p>{/if}

        {#if problems.length}
          <ul class="problems">{#each problems as problem}<li>{problem}</li>{/each}</ul>
        {/if}

        <div class="actions">
          <button class="button primary" disabled={busy || problems.length > 0} onclick={save}>{busy ? 'Checking…' : 'Save and check this computer'}</button>
          {#if !edit}<button class="button ghost" onclick={() => door = 'choose'}>Back</button>{/if}
          {#if oncancel}<button class="button ghost" onclick={oncancel}>Close</button>{/if}
        </div>

        {#if state_}
          <section class="checks-section">
            <h2>What this computer still needs</h2>
            <ul class="checks">
              {#each state_.requirements as item}
                <li class:met={item.satisfied}>
                  <strong>{item.satisfied ? '✓' : '•'} {item.label}</strong>
                  <span>{item.detail}</span>
                  {#if !item.satisfied && item.fix}<span class="fix">{item.fix}</span>{/if}
                </li>
              {/each}
            </ul>
            <button class="button ghost" onclick={refresh}>Check again</button>
          </section>
        {/if}
      </div>
    {/if}
  </div>
</section>

<style>
.setup{position:absolute;inset:0;overflow:auto;background:var(--bg-base);}
.setup-inner{max-width:760px;margin:0 auto;padding:2.5rem 1.5rem 4rem;display:grid;gap:1.5rem;}
header h1{font-size:1.6rem;}
header p,.muted{color:var(--text-secondary);line-height:1.55;}
.muted{font-size:.9rem;}
.doors{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:1rem;}
.door{display:flex;flex-direction:column;gap:.5rem;text-align:left;padding:1.25rem;border:1px solid var(--border-panel);border-radius:var(--radius);background:var(--bg-panel);min-height:11rem;}
.door:hover:not(:disabled){border-color:var(--brand);background:var(--bg-active);}
.door:disabled{opacity:.6;}
.door h2{font-size:1.05rem;}
.door p{font-size:.9rem;color:var(--text-secondary);line-height:1.5;}
.door-go{margin-top:auto;font-size:.85rem;font-weight:600;color:var(--brand);}
.questions{display:grid;gap:1.75rem;}
.questions section{display:grid;gap:.6rem;}
.questions h2{font-size:1.05rem;}
.path-row{display:flex;gap:.5rem;}
.path-row input{flex:1;min-width:0;}
.subject{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr) auto auto;gap:.5rem;align-items:center;}
.subject .advanced{grid-column:1/-1;}
.advanced summary{font-size:.85rem;color:var(--text-secondary);}
.advanced label{display:block;margin:.5rem 0 .25rem;font-weight:600;font-size:.85rem;}
.advanced input{width:100%;}
.problems,.checks{display:grid;gap:.5rem;list-style:none;padding:0;}
.problems li{color:var(--accent-yellow);font-size:.9rem;}
.problem{color:var(--accent-red);font-size:.85rem;}
.saved{color:var(--accent-green);font-size:.9rem;}
.checks li{display:grid;gap:.15rem;padding:.6rem .75rem;border:1px solid var(--border-panel);border-radius:var(--radius);background:var(--bg-panel);}
.checks li.met strong{color:var(--accent-green);}
.checks span{font-size:.85rem;color:var(--text-secondary);overflow-wrap:anywhere;}
.checks .fix{color:var(--accent-blue);}
.checks-section{border-top:1px solid var(--border-panel);padding-top:1.25rem;}
.actions{display:flex;gap:.5rem;flex-wrap:wrap;}
.add-subject{justify-self:start;}
.waiting,.done{display:grid;gap:.75rem;justify-items:stretch;padding:1.5rem;border:1px solid var(--border-panel);border-radius:var(--radius);background:var(--bg-panel);}
.waiting .button,.done .button{justify-self:start;}
.command-row{display:flex;gap:.5rem;align-items:center;width:100%;}
.command-row code{flex:1;min-width:0;overflow-wrap:anywhere;font-family:var(--font-mono);font-size:.85rem;background:var(--bg-surface);padding:.5rem .6rem;border-radius:6px;}
.pulse{display:flex;gap:.4rem;}
.pulse span{width:.5rem;height:.5rem;border-radius:50%;background:var(--brand);animation:pulse 1.2s infinite ease-in-out;}
.pulse span:nth-child(2){animation-delay:.2s;}
.pulse span:nth-child(3){animation-delay:.4s;}
@keyframes pulse{0%,100%{opacity:.25;}50%{opacity:1;}}
@media(prefers-reduced-motion:reduce){.pulse span{animation:none;opacity:.7;}}
@media(max-width:700px){.doors{grid-template-columns:1fr;}.subject{grid-template-columns:1fr;}}
</style>
