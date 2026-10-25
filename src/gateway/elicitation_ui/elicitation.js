// TKS Intent Elicitation Studio Front-End Logic (Track E-A, D-130..D-134, §6.3, §6.4).

(function () {
  'use strict';

  // State
  let currentSession = null;
  let sseSource = null;
  let asrWorker = null;
  let asrReady = false;
  let audioContext = null;
  let audioStream = null;
  let workletNode = null;
  let isRecording = false;
  let isFlushingRecording = false;
  let stopHangoverTimer = null;
  let flushFallbackTimer = null;
  let recordingTarget = 'main'; // 'main' or 'topic'
  let draftHasAsr = false;
  let currentProposalId = null;
  let currentProposalText = null;
  let activeSidecarTab = 'inspiration'; // 'inspiration' or 'socratic'
  let excludedIdeas = [];
  let skipNextAutoBrainstorm = false;

  // DOM Elements - Left Sidebar
  const setupCard = document.getElementById('setup-card');
  const sessionInfoCard = document.getElementById('session-info-card');
  const gapQuestionsCard = document.getElementById('gap-questions-card');
  const gapQuestionsList = document.getElementById('gap-questions-list');
  const createForm = document.getElementById('create-session-form');

  const metaSessionId = document.getElementById('meta-session-id');
  const metaCycle = document.getElementById('meta-cycle');
  const metaDate = document.getElementById('meta-date');
  const metaStakeholder = document.getElementById('meta-stakeholder');
  const metaFacilitator = document.getElementById('meta-facilitator');
  const metaSource = document.getElementById('meta-source');
  const sessionStateBadge = document.getElementById('session-state-badge');

  const btnCloseSession = document.getElementById('btn-close-session');
  const btnReopenSession = document.getElementById('btn-reopen-session');
  const btnExportTranscript = document.getElementById('btn-export-transcript');
  const btnDeleteSession = document.getElementById('btn-delete-session');
  const btnAskOpening = document.getElementById('btn-ask-opening');

  // Center Timeline Pane & Dock
  const timelineTitle = document.getElementById('timeline-session-title');
  const turnCountBadge = document.getElementById('turn-count-badge');
  const turnsList = document.getElementById('turns-list');

  const partialBar = document.getElementById('partial-bar');
  const partialText = document.getElementById('partial-text');
  const draftBox = document.getElementById('draft-box');
  const composerRole = document.getElementById('composer-role');
  const draftModeBadge = document.getElementById('draft-mode-badge');
  const autoSubmitToggle = document.getElementById('auto-submit-toggle');
  const autoProposeToggle = document.getElementById('auto-propose-toggle');
  const autoProposeLabel = document.getElementById('auto-propose-label');
  const micBtn = document.getElementById('mic-btn');
  const micStatusLabel = document.getElementById('mic-status-label');
  const btnClearDraft = document.getElementById('btn-clear-draft');
  const btnSubmitTurn = document.getElementById('btn-submit-turn');
  const toastMsg = document.getElementById('toast-msg');

  // Dynamic Stream Composer elements
  const chatDock = document.getElementById('chat-dock');
  const composerTargetLabel = document.getElementById('composer-target-label');
  const btnCancelInlineInsert = document.getElementById('btn-cancel-inline-insert');
  let pendingInsertAfterTurn = null;
  let activeProbeTurnNumber = null;
  let activeProbeProposalId = null;

  // Right Sidecar Panel
  const sidecarModelTag = document.getElementById('sidecar-model-tag');
  const tabInspiration = document.getElementById('tab-inspiration');
  const tabSocratic = document.getElementById('tab-socratic');
  const secInspiration = document.getElementById('sec-inspiration');
  const secSocratic = document.getElementById('sec-socratic');

  const topicInput = document.getElementById('topic-input');
  const btnClearTopic = document.getElementById('btn-clear-topic');
  const micTopicBtn = document.getElementById('mic-topic-btn');
  const btnBrainstorm = document.getElementById('btn-brainstorm');
  const ideasDeck = document.getElementById('ideas-deck');
  const ideasPlaceholder = document.getElementById('ideas-placeholder');

  const proposalBox = document.getElementById('proposal-box');
  const proposalText = document.getElementById('proposal-text');
  const proposalStatus = document.getElementById('proposal-status');
  const proposalLintBadges = document.getElementById('proposal-lint-badges');
  const btnAnswerProposal = document.getElementById('btn-answer-proposal');
  const btnAcceptProposal = document.getElementById('btn-accept-proposal');
  const btnEditProposal = document.getElementById('btn-edit-proposal');
  const btnRegenProposal = document.getElementById('btn-regen-proposal');
  const btnRejectProposal = document.getElementById('btn-reject-proposal');

  const facilitatorErrorBox = document.getElementById('facilitator-error-box');
  const facilitatorErrorMsg = document.getElementById('facilitator-error-msg');
  const btnRetryProposal = document.getElementById('btn-retry-proposal');
  const btnDismissFacilitatorError = document.getElementById('btn-dismiss-facilitator-error');

  // Capability banner elements
  const asrDot = document.getElementById('asr-dot');
  const asrText = document.getElementById('asr-text');
  const facilitatorDot = document.getElementById('facilitator-dot');
  const facilitatorText = document.getElementById('facilitator-text');

  // Toast helper
  function showToast(message, type = 'error') {
    toastMsg.textContent = message;
    toastMsg.className = `toast show ${type}`;
    setTimeout(() => {
      toastMsg.className = 'toast';
    }, 4000);
  }

  // 1. Intelligent Text Normalizer: disfluency removal + sentence casing
  function cleanTranscriptText(raw) {
    if (!raw) return '';
    let text = raw.trim();

    // Disfluency / filler removal
    text = text.replace(/\b(um+|uh+|er+|ah+|erm+)\b/gi, '');

    // Immediate stutter word deduplication (e.g. "the the" -> "the")
    let prev;
    do {
      prev = text;
      text = text.replace(/\b([a-zA-Z]+)\s+\1\b/gi, '$1');
    } while (text !== prev);

    // Normalize multiple whitespace
    text = text.replace(/\s+/g, ' ').trim();
    if (!text) return '';

    // Lowercase words, preserving uppercase for known acronyms and pronoun 'I'
    let words = text.toLowerCase().split(' ');
    const acronyms = new Set(['ui', 'api', 'ai', 'tks', 'cli', 'rest', 'sse', 'http', 'https', 'wasm', 'html', 'css', 'db', 'sql']);

    for (let i = 0; i < words.length; i++) {
      let w = words[i];
      let match = w.match(/^([a-z\x27\u2019]+)([.,!?;:]*)$/);
      if (match) {
        let core = match[1];
        let punct = match[2];
        if (acronyms.has(core)) {
          words[i] = core.toUpperCase() + punct;
        } else if (core === 'i' || core.startsWith("i\x27") || core.startsWith("i\u2019")) {
          words[i] = 'I' + core.slice(1) + punct;
        }
      }
    }
    text = words.join(' ');

    // Capitalize first letter of each sentence
    text = text.replace(/(^\s*|[.!?]\s+)([a-z])/g, (m, p1, p2) => p1 + p2.toUpperCase());

    return text;
  }

  // Parse structured ideas from inspiration text, including optional [Gap: ...] or [Elaborates ...] tags
  function parseInspirationIdeas(text) {
    if (!text) return [];
    const rawSections = text.split(/\n\s*\n/).map((s) => s.trim()).filter(Boolean);
    const ideas = [];
    for (const sec of rawSections) {
      const titleMatch = sec.match(/^(?:\d+\.|\*|-)?\s*(?:\*\*(.*?)\*\*|([A-Z][\w\s[\]:-]{2,60}):)\s*([\s\S]*)$/);
      if (titleMatch) {
        const rawTitle = (titleMatch[1] || titleMatch[2] || '').trim().replace(/:$/, '');
        const body = (titleMatch[3] || '').trim().replace(/^:\s*/, '');
        if (rawTitle && body) {
          let gap = null;
          let title = rawTitle;
          const bracketMatch = rawTitle.match(/^\[([^\]]+)\]\s*(.+)$/);
          if (bracketMatch) {
            gap = bracketMatch[1].trim();
            title = bracketMatch[2].trim();
          }
          ideas.push({
            gap,
            title,
            rawTitle,
            body,
            fullText: `${title}: ${body}`,
          });
          continue;
        }
      }
      const fallbackTitle = sec.split(/[.\n]/)[0].slice(0, 48).trim();
      ideas.push({
        gap: null,
        title: fallbackTitle,
        rawTitle: fallbackTitle,
        body: sec,
        fullText: sec,
      });
    }
    return ideas;
  }

  function recordExcludedIdea(itemText) {
    if (!itemText) return;
    const cleaned = itemText.trim();
    if (!cleaned) return;
    if (!excludedIdeas.some((existing) => existing.toLowerCase() === cleaned.toLowerCase())) {
      excludedIdeas.push(cleaned);
    }
  }

  function getParticipantName(session, role) {
    const fallback = role === 'stakeholder' ? 'Stakeholder' : 'Facilitator';
    if (!session || !session.participants) return fallback;
    if (Array.isArray(session.participants)) {
      const found = session.participants.find((p) => p.role === role);
      return (found && found.name) || fallback;
    }
    return session.participants[role]?.name || fallback;
  }

  // Check WASM SIMD capability probe (using standard wasm-feature-detect bytecode)
  function checkSimdSupport() {
    try {
      return WebAssembly.validate(new Uint8Array([
        0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 
        3, 2, 1, 0, 10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11
      ]));
    } catch (_) {
      return false;
    }
  }

  // Initialize Capabilities & ASR Worker
  async function initCapabilities() {
    try {
      const resp = await fetch('/api/v1/elicitation/capabilities');
      if (!resp.ok) {
        throw new Error(`HTTP ${resp.status}`);
      }
      const caps = await resp.json();

      // Facilitator status
      if (caps.facilitator && (
        caps.facilitator.provider === 'ollama' ||
        (caps.facilitator.provider === 'tierfall' && caps.facilitator.available)
      )) {
        const isTierfall = caps.facilitator.provider === 'tierfall';
        const modelName = isTierfall ? caps.facilitator.route : caps.facilitator.model;
        facilitatorDot.className = 'dot green';
        facilitatorText.textContent = `Facilitator: ${isTierfall ? 'Tierfall (hosted routing)' : 'Ollama'} (${modelName})`;
        sidecarModelTag.textContent = modelName;
        autoProposeToggle.checked = true;
        autoProposeLabel.classList.remove('hidden');
      } else {
        facilitatorDot.className = 'dot amber';
        const unavailable = caps.facilitator && caps.facilitator.provider === 'tierfall';
        facilitatorText.textContent = unavailable ? 'Facilitator: Tierfall (Unavailable)' : 'Facilitator: None (Manual)';
        sidecarModelTag.textContent = unavailable ? 'Unavailable' : 'Manual';
        autoProposeToggle.checked = false;
        autoProposeLabel.classList.add('hidden');
      }

      // ASR status
      const simdSupported = checkSimdSupport();
      if (!caps.asr || !caps.asr.available) {
        asrDot.className = 'dot amber';
        asrText.textContent = 'ASR: Disabled (Typed only)';
        micStatusLabel.textContent = 'ASR not configured (Typed mode only)';
        return;
      }

      if (!simdSupported) {
        asrDot.className = 'dot red';
        asrText.textContent = 'ASR: Browser lacks SIMD';
        micStatusLabel.textContent = 'WASM SIMD unsupported (Typed mode only)';
        return;
      }

      asrDot.className = 'dot green';
      asrText.textContent = `ASR: Ready (${caps.asr.version})`;
      micStatusLabel.textContent = 'Downloading speech model...';

      // Initialize WebWorker
      initAsrWorker(caps.asr.assets_base, caps.asr.version);
    } catch (err) {
      asrDot.className = 'dot red';
      asrText.textContent = 'ASR: Offline';
      micStatusLabel.textContent = 'Failed to load capabilities';
      console.warn('Elicitation capabilities query failed:', err);
    }
  }

  function initAsrWorker(assetsBase, version) {
    try {
      asrWorker = new Worker('/explorer/elicitation/asr-worker.js');
      const fullAssetsBase = `${assetsBase.replace(/\/$/, '')}/${version}/`;

      asrWorker.onmessage = function (e) {
        const { type } = e.data;
        if (type === 'status') {
          if (!asrReady && e.data.text) {
            micStatusLabel.textContent = e.data.text;
          }
        } else if (type === 'ready') {
          asrReady = true;
          micBtn.disabled = false;
          micTopicBtn.disabled = false;
          micStatusLabel.textContent = 'Ready (Hold Ctrl+Alt+V or Mic to speak)';
        } else if (type === 'partial') {
          if (isRecording || isFlushingRecording) {
            const liveText = e.data.text || '...';
            if (recordingTarget === 'topic') {
              topicInput.value = cleanTranscriptText(liveText) || liveText;
            } else {
              const display = cleanTranscriptText(liveText) || liveText;
              partialText.textContent = isFlushingRecording
                ? `Finishing transcription: ${display}`
                : display;
            }
          }
        } else if (type === 'final') {
          isFlushingRecording = false;
          micBtn.classList.remove('recording', 'flushing');
          micTopicBtn.classList.remove('recording');
          partialBar.classList.remove('recording', 'flushing');
          if (asrReady) {
            micStatusLabel.textContent = 'Ready (Hold Ctrl+Alt+V or Mic to speak)';
          }

          if (e.data.memoryBytes) {
            window.testLastMem = e.data.memoryBytes;
          }
          const raw = (e.data.text || '').trim();
          const cleaned = cleanTranscriptText(raw);

          if (recordingTarget === 'topic') {
            if (cleaned) {
              topicInput.value = cleaned;
              updateTopicControlsUI();
              requestInspiration(cleaned, { isExplicitTopic: true });
            } else {
              topicInput.value = '';
              updateTopicControlsUI();
            }
          } else {
            partialText.textContent = 'Push-to-talk: Hold Ctrl+Alt+V or Mic button to record spoken intent';

            if (cleaned) {
              if (autoSubmitToggle.checked) {
                // Power-user mode: auto-commit directly to timeline after full tail flush
                submitStakeholderTurn(cleaned, 'dictated');
              } else {
                // Review mode: place in draft box
                draftBox.value = cleaned;
                draftHasAsr = true;
                updateDraftModeUI();
                draftBox.focus();
              }
            }
          }
        } else if (type === 'error') {
          console.error('ASR Worker error:', e.data.error);
          showToast(`ASR Error: ${e.data.error}`);
          micStatusLabel.textContent = 'ASR Worker Error';
        }
      };

      asrWorker.postMessage({ type: 'init', assetsBase: fullAssetsBase });
    } catch (err) {
      console.error('Failed to create ASR WebWorker:', err);
      micStatusLabel.textContent = 'WebWorker initialization failed';
    }
  }

  // Audio Recording & AudioWorklet lifecycle
  async function startRecording(target = 'main') {
    if (isRecording || isFlushingRecording || !asrWorker || micBtn.disabled) return;
    if (currentSession && currentSession.state === 'closed') return;

    recordingTarget = target;

    try {
      audioStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          sampleRate: 16000,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });

      audioContext = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 16000 });
      await audioContext.audioWorklet.addModule('/explorer/elicitation/audio-worklet.js');

      const sourceNode = audioContext.createMediaStreamSource(audioStream);
      workletNode = new AudioWorkletNode(audioContext, 'audio-recorder-processor');

      workletNode.port.onmessage = function (e) {
        if (!isRecording && !isFlushingRecording) return;
        if (e.data.type === 'pcm') {
          asrWorker.postMessage({ type: 'pcm', data: e.data.data }, [e.data.data.buffer]);
        } else if (e.data.type === 'flushed') {
          finalizeAudioTeardownAndStopWorker(false);
        }
      };

      sourceNode.connect(workletNode);
      workletNode.connect(audioContext.destination);

      isRecording = true;
      isFlushingRecording = false;
      if (recordingTarget === 'topic') {
        micTopicBtn.classList.add('recording');
        topicInput.value = 'Listening...';
      } else {
        micBtn.classList.add('recording');
        partialBar.classList.remove('flushing');
        partialBar.classList.add('recording');
        partialText.textContent = 'Listening...';
      }

      asrWorker.postMessage({ type: 'start' });
    } catch (err) {
      console.error('getUserMedia failed:', err);
      showToast(`Microphone access error: ${err.message}`);
      stopRecording(true);
    }
  }

  function finalizeAudioTeardownAndStopWorker(forceDiscard = false) {
    if (stopHangoverTimer) {
      clearTimeout(stopHangoverTimer);
      stopHangoverTimer = null;
    }
    if (flushFallbackTimer) {
      clearTimeout(flushFallbackTimer);
      flushFallbackTimer = null;
    }

    if (workletNode) {
      try { workletNode.disconnect(); } catch (_) {}
      workletNode = null;
    }

    if (audioStream) {
      audioStream.getTracks().forEach((track) => {
        try { track.stop(); } catch (_) {}
      });
      audioStream = null;
    }

    if (audioContext && audioContext.state !== 'closed') {
      try { audioContext.close(); } catch (_) {}
      audioContext = null;
    }

    if (forceDiscard) {
      isFlushingRecording = false;
      micBtn.classList.remove('recording', 'flushing');
      micTopicBtn.classList.remove('recording');
      partialBar.classList.remove('recording', 'flushing');
    } else if (asrWorker) {
      asrWorker.postMessage({ type: 'stop' });
    }
  }

  function stopRecording(forceDiscard = false) {
    if (!isRecording && !isFlushingRecording) return;
    isRecording = false;

    if (forceDiscard) {
      finalizeAudioTeardownAndStopWorker(true);
      return;
    }

    if (isFlushingRecording) return;
    isFlushingRecording = true;

    micBtn.classList.remove('recording');
    if (recordingTarget === 'main') {
      micBtn.classList.add('flushing');
      partialBar.classList.remove('recording');
      partialBar.classList.add('flushing');
      partialText.textContent = 'Finishing transcription...';
    }
    micStatusLabel.textContent = 'Finishing transcription...';

    // Allow 260 ms of post-release audio hangover so the OS/browser input pipeline
    // drains the speaker's final syllable before flushing the AudioWorklet buffer.
    stopHangoverTimer = setTimeout(() => {
      stopHangoverTimer = null;
      if (workletNode && workletNode.port) {
        try {
          workletNode.port.postMessage({ type: 'flush' });
          flushFallbackTimer = setTimeout(() => {
            finalizeAudioTeardownAndStopWorker(false);
          }, 120);
          return;
        } catch (_) {}
      }
      finalizeAudioTeardownAndStopWorker(false);
    }, 260);
  }

  // Session Form Submission
  createForm.addEventListener('submit', async (e) => {
    e.preventDefault();
    const title = document.getElementById('setup-title').value.trim();
    const sessionId = document.getElementById('setup-slug').value.trim();
    const stakeholderName = document.getElementById('setup-stakeholder').value.trim();
    const facilitatorName = document.getElementById('setup-facilitator').value.trim();

    const priorTranscript = document.getElementById('setup-prior-transcript').value.trim();
    const priorExtractionId = document.getElementById('setup-prior-extraction-id').value.trim();

    const btnSubmit = document.getElementById('btn-create-session');
    btnSubmit.disabled = true;
    btnSubmit.textContent = 'Creating Session...';

    const payload = {
      title,
      session_id: sessionId,
      stakeholder_name: stakeholderName,
      facilitator_name: facilitatorName,
      source: 'typed',
      prior_transcript: priorTranscript || undefined,
      prior_extraction_id: priorExtractionId || undefined,
    };

    try {
      const resp = await fetch('/api/v1/elicitation/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });

      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.message || `Failed to create session (${resp.status})`);
      }

      const session = await resp.json();
      currentSession = session;
      loadSession(session);
      showToast('Elicitation session initialized.', 'success');
    } catch (err) {
      showToast(err.message);
    } finally {
      btnSubmit.disabled = false;
      btnSubmit.textContent = 'Start Elicitation Session';
    }
  });

  function loadSession(session) {
    window.__currentSession = session;
    excludedIdeas = Array.isArray(session.explored_items) ? [...session.explored_items] : [];
    skipNextAutoBrainstorm = false;
    setupCard.classList.add('hidden');
    sessionInfoCard.classList.remove('hidden');

    metaSessionId.textContent = session.session_id || session.elicitation_id;
    metaCycle.textContent = session.cycle || 1;
    metaDate.textContent = session.date || new Date().toISOString().split('T')[0];
    metaStakeholder.textContent = getParticipantName(session, 'stakeholder');
    metaFacilitator.textContent = getParticipantName(session, 'facilitator');
    metaSource.textContent = session.source || 'manual';

    timelineTitle.textContent = session.title || session.session_id;
    updateSessionStateUI();

    // Populate Gap Questions if present
    if (session.imported_gap_questions && session.imported_gap_questions.length > 0) {
      gapQuestionsCard.classList.remove('hidden');
      gapQuestionsList.innerHTML = '';
      session.imported_gap_questions.forEach((q) => {
        const item = document.createElement('div');
        item.className = 'gap-item';
        item.textContent = `❓ ${q.suggested_question}`;
        item.title = `Gap from prior cycle (${q.detector || q.category || 'gap'})`;
        item.addEventListener('click', () => {
          topicInput.value = q.suggested_question;
          updateTopicControlsUI();
          requestInspiration(q.suggested_question, { isExplicitTopic: true });
        });
        gapQuestionsList.appendChild(item);
      });
    } else {
      gapQuestionsCard.classList.add('hidden');
    }

    renderTurns(session.turns || []);
    connectSessionSSE(session.elicitation_id);

    if ((!session.turns || session.turns.length === 0) && autoProposeToggle.checked) {
      btnAskOpening.classList.remove('hidden');
    } else {
      btnAskOpening.classList.add('hidden');
    }
  }

  function updateSessionStateUI() {
    if (!currentSession) return;
    const isClosed = currentSession.state === 'closed';

    sessionStateBadge.textContent = isClosed ? 'Closed' : 'Open';
    sessionStateBadge.classList.toggle('state-badge-closed', isClosed);
    sessionStateBadge.classList.toggle('state-badge-open', !isClosed);

    btnCloseSession.classList.toggle('hidden', isClosed);
    btnReopenSession.classList.toggle('hidden', !isClosed);
    btnExportTranscript.disabled = !isClosed;
    btnSubmitTurn.disabled = isClosed || draftBox.value.trim().length === 0;
    micBtn.disabled = isClosed;
    micTopicBtn.disabled = isClosed;
  }

  function updateTurnCounter() {
    if (turnCountBadge && currentSession) {
      const count = currentSession.turns ? currentSession.turns.length : 0;
      turnCountBadge.textContent = `${count} ${count === 1 ? 'turn' : 'turns'}`;
    }
  }

  function connectSessionSSE(sessionId) {
    if (sseSource) {
      sseSource.close();
      sseSource = null;
    }

    sseSource = new EventSource(`/api/v1/elicitation/sessions/${sessionId}/events`);

    sseSource.addEventListener('turn_committed', (e) => {
      const raw = JSON.parse(e.data);
      const turn = raw.data || raw;
      appendTurnToTimeline(turn);
      if (currentSession && turn.input_mode === 'dictated') {
        currentSession.source = 'asr';
      }
      metaSource.textContent = currentSession ? currentSession.source : '-';

      if (skipNextAutoBrainstorm) {
        skipNextAutoBrainstorm = false;
        return;
      }

      // Auto-trigger assistant when active and in open session
      if (turn.role === 'stakeholder' && autoProposeToggle.checked && currentSession && currentSession.state === 'open') {
        if (activeSidecarTab === 'inspiration') {
          requestInspiration('', { isExplicitTopic: false });
        } else {
          requestFacilitatorProposal();
        }
      }
    });

    sseSource.addEventListener('turn_updated', (e) => {
      const raw = JSON.parse(e.data);
      const turn = raw.data || raw;
      updateTurnInTimeline(turn);
    });

    sseSource.addEventListener('turn_inserted', (e) => {
      const raw = JSON.parse(e.data);
      const data = raw.data || raw;
      if (currentSession) {
        currentSession.turns = data.turns || [];
        if (data.source) {
          currentSession.source = data.source;
          metaSource.textContent = data.source;
        }
        renderTurns(currentSession.turns);
      }
    });

    sseSource.addEventListener('turn_deleted', (e) => {
      const raw = JSON.parse(e.data);
      const data = raw.data || raw;
      if (currentSession) {
        currentSession.turns = data.turns || [];
        if (data.source) {
          currentSession.source = data.source;
          metaSource.textContent = data.source;
        }
        renderTurns(currentSession.turns);
      }
    });

    sseSource.addEventListener('facilitator_proposal', (e) => {
      const raw = JSON.parse(e.data);
      const data = raw.data || raw;
      if (data.generation) {
        const g = data.generation;
        sidecarModelTag.textContent = `${g.provider} / ${g.model} · ${(g.duration_ms / 1000).toFixed(1)}s`;
        sidecarModelTag.title = `${g.account || 'direct'}; ${g.attempts} upstream attempt(s)`;
      }
      showProposal(data);
    });

    sseSource.addEventListener('facilitator_error', (e) => {
      const raw = JSON.parse(e.data);
      const data = raw.data || raw;
      showFacilitatorError(data.error);
    });

    sseSource.addEventListener('session_closed', () => {
      if (currentSession) {
        currentSession.state = 'closed';
        updateSessionStateUI();
      }
    });

    sseSource.onerror = () => {
      if (currentSession) {
        fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}`)
          .then((r) => r.json())
          .then((s) => {
            currentSession = s;
            loadSession(s);
          })
          .catch(() => {});
      }
    };
  }

  // Turn Submission
  // Turn Submission
  async function submitTurn(role, rawText, inputMode = 'typed') {
    if (!currentSession || currentSession.state === 'closed') return;
    const cleaned = cleanTranscriptText(rawText);
    if (!cleaned) return;

    btnSubmitTurn.disabled = true;
    try {
      const payload = {
        role,
        text: cleaned,
        input_mode: inputMode,
      };
      if (pendingInsertAfterTurn !== null) {
        payload.insert_after_turn = pendingInsertAfterTurn;
      }

      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });

      if (!resp.ok) {
        const errJson = await resp.json().catch(() => ({}));
        throw new Error(errJson.message || `HTTP ${resp.status}`);
      }

      clearDraft();
      btnAskOpening.classList.add('hidden');
      hideProposal();
      cancelInlineInsert();
    } catch (err) {
      showToast(`Failed to submit turn: ${err.message}`);
      btnSubmitTurn.disabled = false;
    }
  }

  async function submitStakeholderTurn(rawText, inputMode = 'typed') {
    return submitTurn('stakeholder', rawText, inputMode);
  }

  async function submitCurrentTurn() {
    const text = draftBox.value.trim();
    if (!text) return;
    const mode = draftHasAsr ? 'dictated' : 'typed';
    const role = (composerRole && composerRole.value) || 'stakeholder';
    await submitTurn(role, text, mode);
  }

  function renderTurns(turns) {
    turnsList.innerHTML = '';
    if (!turns || turns.length === 0) {
      turnsList.innerHTML = '<div class="empty-state">No turns recorded yet. Speak or type to begin.</div>';
      updateTurnCounter();
      repositionComposer();
      return;
    }

    turns.forEach((turn) => {
      appendTurnToTimeline(turn);
    });
    repositionComposer();
  }

  function appendTurnToTimeline(turn) {
    if (turnsList.children.length === 1 && turnsList.children[0].textContent.includes('No turns')) {
      turnsList.innerHTML = '';
    }

    if (currentSession) {
      if (!currentSession.turns) currentSession.turns = [];
      if (!currentSession.turns.find((t) => t.number === turn.number)) {
        currentSession.turns.push(turn);
      }
    }

    const participantName = getParticipantName(currentSession, turn.role);
    const isOutOfScope = turn.text.trim().toLowerCase().startsWith('out of scope');

    const item = document.createElement('div');
    item.className = `turn-item ${turn.role}${isOutOfScope ? ' out-of-scope' : ''}`;
    item.id = `turn-${turn.number}`;

    const uncertaintyHtml = turn.uncertain_flags && turn.uncertain_flags.length > 0
      ? `<div class="turn-uncertainty-flag" id="turn-flag-${turn.number}">⚠️ Uncertain: ${turn.uncertain_flags.map((f) => f.note || 'Needs review').join(', ')}</div>`
      : '';

    const roleBadgeHtml = isOutOfScope
      ? '<span class="turn-role-badge out-of-scope">Out of Scope</span>'
      : `<span class="turn-role-badge ${turn.role}">${turn.role}</span>`;

    const probeBtnHtml = (turn.role === 'stakeholder' && !isOutOfScope)
      ? `<button class="btn btn-secondary btn-sm btn-probe-turn" data-turn="${turn.number}" title="Ask Socratic bisection question regarding this statement">🔍 Probe</button>`
      : '';

    item.innerHTML = `
      <div class="turn-item-header">
        <div class="turn-info">
          <span class="turn-number">[t${turn.number}]</span>
          <strong>${escapeHtml(participantName)}</strong>
          ${roleBadgeHtml}
          <span class="turn-mode-badge">${turn.input_mode}</span>
        </div>
        <div class="turn-actions">
          <button class="btn btn-secondary btn-sm btn-edit-turn" data-turn="${turn.number}">Edit</button>
          <button class="btn btn-secondary btn-sm btn-flag-turn" data-turn="${turn.number}">${turn.uncertain_flags && turn.uncertain_flags.length > 0 ? 'Clear Flag' : 'Flag Uncertain'}</button>
          ${probeBtnHtml}
          <button class="btn btn-danger btn-sm btn-delete-turn" data-turn="${turn.number}" title="Delete this statement">Delete</button>
        </div>
      </div>
      <div class="turn-text" id="turn-text-${turn.number}">${escapeHtml(turn.text)}</div>
      ${uncertaintyHtml}
    `;

    const btnEdit = item.querySelector('.btn-edit-turn');
    const btnFlag = item.querySelector('.btn-flag-turn');
    const btnDelete = item.querySelector('.btn-delete-turn');
    const btnProbe = item.querySelector('.btn-probe-turn');

    btnEdit.addEventListener('click', () => handleCorrectTurn(turn.number));
    btnFlag.addEventListener('click', () => handleFlagTurn(turn.number));
    btnDelete.addEventListener('click', () => handleDeleteTurn(turn.number));
    if (btnProbe) {
      btnProbe.addEventListener('click', () => handleProbeTurn(turn.number, turn.text));
    }

    turnsList.appendChild(item);
    repositionComposer();
    turnsList.scrollTop = turnsList.scrollHeight;
    updateTurnCounter();
  }

  function removeActiveProbeBox() {
    const existing = document.getElementById('turn-probe-box');
    if (existing) existing.remove();
    activeProbeTurnNumber = null;
    activeProbeProposalId = null;
  }

  async function handleProbeTurn(turnNumber, turnText) {
    if (!currentSession || currentSession.state === 'closed') return;

    // Toggle off if clicking the currently probed turn
    if (pendingInsertAfterTurn === turnNumber && activeProbeTurnNumber === turnNumber) {
      cancelInlineInsert();
      return;
    }

    removeActiveProbeBox();
    pendingInsertAfterTurn = turnNumber;
    activeProbeTurnNumber = turnNumber;

    const targetTurnEl = document.getElementById(`turn-${turnNumber}`);
    if (!targetTurnEl) return;

    const probeBox = document.createElement('div');
    probeBox.className = 'turn-probe-box';
    probeBox.id = 'turn-probe-box';
    probeBox.innerHTML = `
      <div class="turn-probe-header">
        <span>❓ Socratic Probe on [t${turnNumber}]</span>
        <div class="flex gap-1">
          <button class="btn btn-secondary btn-xs btn-regen-probe">Next Question</button>
          <button class="btn btn-secondary btn-xs btn-close-probe" title="Close probe">✕</button>
        </div>
      </div>
      <div class="turn-probe-text" id="turn-probe-text">Formulating Socratic question on operational boundaries...</div>
    `;

    targetTurnEl.after(probeBox);
    repositionComposer();

    probeBox.querySelector('.btn-close-probe').addEventListener('click', () => {
      cancelInlineInsert();
    });

    const triggerProbeQuery = async () => {
      const probeTextEl = document.getElementById('turn-probe-text');
      if (probeTextEl) probeTextEl.textContent = 'Thinking of Socratic bisection question...';
      try {
        const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/facilitator/proposals`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            mode: 'socratic',
            topic: turnText,
            excluded_ideas: excludedIdeas,
          }),
        });
        if (!resp.ok) {
          const err = await resp.json().catch(() => ({}));
          throw new Error(err.message || `HTTP ${resp.status}`);
        }
        const data = await resp.json();
        if (data && data.proposal_id) {
          activeProbeProposalId = data.proposal_id;
        }
      } catch (err) {
        const pt = document.getElementById('turn-probe-text');
        if (pt) pt.textContent = `Error: ${err.message}`;
      }
    };

    probeBox.querySelector('.btn-regen-probe').addEventListener('click', () => {
      triggerProbeQuery();
    });

    await triggerProbeQuery();
    draftBox.focus();
  }

  function cancelInlineInsert() {
    pendingInsertAfterTurn = null;
    activeProbeTurnNumber = null;
    activeProbeProposalId = null;
    removeActiveProbeBox();
    repositionComposer();
  }

  function repositionComposer() {
    if (!chatDock || !turnsList) return;

    if (pendingInsertAfterTurn !== null) {
      const probeBox = document.getElementById('turn-probe-box');
      const targetTurnEl = document.getElementById(`turn-${pendingInsertAfterTurn}`);
      const anchor = probeBox || targetTurnEl;
      if (anchor) {
        anchor.after(chatDock);
        chatDock.classList.add('inline-insert');
        if (composerTargetLabel) {
          composerTargetLabel.innerHTML = `<span class="badge" style="background:#a855f7;color:white;">Clarifying after [t${pendingInsertAfterTurn}]</span>`;
        }
        if (btnCancelInlineInsert) {
          btnCancelInlineInsert.classList.remove('hidden');
        }
        return;
      }
    }

    // Default tail position
    turnsList.appendChild(chatDock);
    chatDock.classList.remove('inline-insert');
    const nextNumber = currentSession && currentSession.turns ? currentSession.turns.length + 1 : 1;
    if (composerTargetLabel) {
      composerTargetLabel.innerHTML = `<span class="badge">Next Statement [t${nextNumber}]</span>`;
    }
    if (btnCancelInlineInsert) {
      btnCancelInlineInsert.classList.add('hidden');
    }
  }

  function updateTurnInTimeline(turn) {
    const existing = document.getElementById(`turn-${turn.number}`);
    if (!existing) return;

    if (currentSession && currentSession.turns) {
      const idx = currentSession.turns.findIndex((t) => t.number === turn.number);
      if (idx !== -1) currentSession.turns[idx] = turn;
    }

    const textEl = document.getElementById(`turn-text-${turn.number}`);
    if (textEl) textEl.textContent = turn.text;

    const oldFlag = document.getElementById(`turn-flag-${turn.number}`);
    if (oldFlag) oldFlag.remove();

    if (turn.uncertain_flags && turn.uncertain_flags.length > 0) {
      const flagDiv = document.createElement('div');
      flagDiv.className = 'turn-uncertainty-flag';
      flagDiv.id = `turn-flag-${turn.number}`;
      flagDiv.textContent = `⚠️ Uncertain: ${turn.uncertain_flags.map((f) => f.note || 'Needs review').join(', ')}`;
      existing.appendChild(flagDiv);
    }

    const btnFlag = existing.querySelector('.btn-flag-turn');
    if (btnFlag) {
      btnFlag.textContent = turn.uncertain_flags && turn.uncertain_flags.length > 0 ? 'Clear Flag' : 'Flag Uncertain';
    }
  }

  // Turn Correction (Inline Editor), Flagging & Deletion
  function handleCorrectTurn(turnNumber) {
    if (!currentSession || currentSession.state === 'closed') return;
    const item = document.getElementById(`turn-${turnNumber}`);
    const textEl = document.getElementById(`turn-text-${turnNumber}`);
    if (!item || !textEl) return;

    // Prevent opening duplicate inline editors on the same turn
    if (item.querySelector('.turn-inline-editor')) return;

    const liveTurn = currentSession.turns
      ? currentSession.turns.find((t) => t.number === turnNumber)
      : null;
    const currentText = liveTurn ? liveTurn.text : textEl.textContent;

    textEl.classList.add('hidden');
    const editor = document.createElement('div');
    editor.className = 'turn-inline-editor';
    editor.innerHTML = `
      <textarea class="form-control" rows="2"></textarea>
      <div class="turn-inline-actions">
        <button type="button" class="btn btn-primary btn-xs btn-save-turn">Save</button>
        <button type="button" class="btn btn-secondary btn-xs btn-cancel-turn">Cancel</button>
      </div>
    `;

    const textarea = editor.querySelector('textarea');
    textarea.value = currentText;

    const closeEditor = () => {
      editor.remove();
      textEl.classList.remove('hidden');
    };

    const saveEdit = async () => {
      const newText = textarea.value.trim();
      if (!newText || newText === currentText) {
        closeEditor();
        return;
      }

      try {
        const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns/${turnNumber}`, {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ text: newText }),
        });

        if (!resp.ok) {
          const err = await resp.json().catch(() => ({}));
          throw new Error(err.message || `HTTP ${resp.status}`);
        }

        const updated = await resp.json();
        closeEditor();
        updateTurnInTimeline(updated);
        showToast(`Turn [t${turnNumber}] updated.`, 'success');
      } catch (err) {
        showToast(`Correction failed: ${err.message}`);
      }
    };

    editor.querySelector('.btn-save-turn').addEventListener('click', saveEdit);
    editor.querySelector('.btn-cancel-turn').addEventListener('click', closeEditor);
    textarea.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        saveEdit();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        closeEditor();
      }
    });

    textEl.insertAdjacentElement('afterend', editor);
    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  }

  async function handleDeleteTurn(turnNumber) {
    if (!currentSession || currentSession.state === 'closed') return;

    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns/${turnNumber}`, {
        method: 'DELETE',
      });

      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.message || `HTTP ${resp.status}`);
      }

      const data = await resp.json();
      if (currentSession && Array.isArray(data.turns)) {
        currentSession.turns = data.turns;
        renderTurns(currentSession.turns);
      }
      showToast(`Statement [t${turnNumber}] deleted.`, 'success');
    } catch (err) {
      showToast(`Delete failed: ${err.message}`);
    }
  }

  async function handleFlagTurn(turnNumber) {
    const liveTurn = currentSession && currentSession.turns
      ? currentSession.turns.find((t) => t.number === turnNumber)
      : null;
    const hasFlags = liveTurn && liveTurn.uncertain_flags && liveTurn.uncertain_flags.length > 0;
    let uncertainFlags = [];

    if (!hasFlags) {
      const note = prompt('Enter uncertainty note (e.g., technical ambiguity, unverifiable metric):', 'Needs clarification');
      if (note === null) return;
      uncertainFlags.push({
        start: 0,
        end: liveTurn ? liveTurn.text.length : 0,
        note: note.trim() || 'Needs clarification',
      });
    }

    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns/${turnNumber}`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          uncertain_flags: uncertainFlags,
        }),
      });

      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.message || `HTTP ${resp.status}`);
      }

      const updated = await resp.json();
      updateTurnInTimeline(updated);
      showToast(`Turn [t${turnNumber}] flags updated.`, 'success');
    } catch (err) {
      showToast(`Flag update failed: ${err.message}`);
    }
  }

  function updateTopicControlsUI() {
    const hasTopic = topicInput && topicInput.value.trim().length > 0;
    if (btnClearTopic) {
      btnClearTopic.classList.toggle('hidden', !hasTopic);
    }
    if (btnBrainstorm) {
      btnBrainstorm.textContent = hasTopic ? 'Brainstorm Focus Topic' : 'Spot Gaps & Brainstorm';
    }
  }

  // Assistant & Inspiration Requests
  async function requestInspiration(topic = '', options = {}) {
    if (!currentSession || currentSession.state === 'closed') return;
    const effectiveTopic = options.isExplicitTopic
      ? (topic || topicInput.value.trim())
      : (topic || '');
    const statusLabel = effectiveTopic
      ? `Brainstorming ideas for "${escapeHtml(effectiveTopic)}"...`
      : 'Analyzing transcript to spot gaps & under-elaborated requirements...';
    ideasDeck.innerHTML = `<div class="sidecar-placeholder"><span class="text-blue">${statusLabel}</span></div>`;
    facilitatorErrorBox.classList.add('hidden');

    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/facilitator/proposals`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          mode: 'inspiration',
          topic: effectiveTopic,
          excluded_ideas: excludedIdeas,
        }),
      });

      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.message || `HTTP ${resp.status}`);
      }
    } catch (err) {
      showFacilitatorError(err.message);
    }
  }

  function renderOpeningQuestion(text) {
    removeActiveProbeBox();
    const box = document.createElement('div');
    box.className = 'turn-probe-box';
    box.id = 'turn-probe-box';
    box.innerHTML = `
      <div class="turn-probe-header">
        <span>❓ Facilitator Opening Question</span>
        <button class="btn btn-secondary btn-xs btn-close-probe" title="Close question">✕</button>
      </div>
      <div class="turn-probe-text">${escapeHtml(text)}</div>
    `;
    turnsList.prepend(box);
    box.querySelector('.btn-close-probe').addEventListener('click', () => {
      removeActiveProbeBox();
      repositionComposer();
    });
    repositionComposer();
    draftBox.focus();
    draftBox.placeholder = 'Answering opening question...';
  }

  async function requestFacilitatorProposal() {
    if (!currentSession || currentSession.state === 'closed') return;
    if (proposalBox) {
      proposalBox.classList.remove('hidden');
      if (proposalText) proposalText.textContent = 'Facilitator is analyzing gaps to formulate a Socratic question...';
      if (proposalStatus) proposalStatus.textContent = 'Thinking...';
      if (proposalLintBadges) proposalLintBadges.innerHTML = '';
      if (btnAcceptProposal) btnAcceptProposal.disabled = true;
      if (btnEditProposal) btnEditProposal.disabled = true;
      if (btnRegenProposal) btnRegenProposal.disabled = true;
      if (btnRejectProposal) btnRejectProposal.disabled = true;
    } else {
      renderOpeningQuestion('Facilitator is analyzing domain scope to ask an opening question...');
    }
    if (facilitatorErrorBox) facilitatorErrorBox.classList.add('hidden');

    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/facilitator/proposals`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          mode: 'socratic',
          excluded_ideas: excludedIdeas,
        }),
      });

      if (!resp.ok) {
        const err = await resp.json().catch(() => ({}));
        throw new Error(err.message || `HTTP ${resp.status}`);
      }
    } catch (err) {
      showFacilitatorError(err.message);
    }
  }

  function showProposal(data) {
    currentProposalId = data.proposal_id;
    currentProposalText = data.text;

    // 1. Check if this proposal matches the active Socratic probe
    const isProbe = (activeProbeProposalId && data.proposal_id === activeProbeProposalId)
      || (data.mode === 'socratic' && activeProbeTurnNumber !== null);

    if (isProbe) {
      activeProbeProposalId = null;
      recordExcludedIdea(data.text);
      const pt = document.getElementById('turn-probe-text');
      if (pt) {
        pt.textContent = data.text;
      }
      return;
    }

    // 2. Check if this is an opening or general Socratic prompt (not an inline probe)
    if (data.mode === 'socratic') {
      recordExcludedIdea(data.text);
      if (proposalBox) {
        renderSocraticPrompt(data);
      } else {
        renderOpeningQuestion(data.text);
      }
      return;
    }

    // 3. Otherwise, it is an inspiration proposal for the Inspiration Studio sidecar
    renderInspirationIdeas(data.text);
  }

  function renderInspirationIdeas(text) {
    const ideas = parseInspirationIdeas(text);
    if (ideas.length === 0) {
      ideasDeck.innerHTML = `<div class="sidecar-placeholder">${escapeHtml(text)}</div>`;
      return;
    }

    ideasDeck.innerHTML = '';
    ideas.forEach((idea, idx) => {
      recordExcludedIdea(idea.rawTitle || idea.title);
      const card = document.createElement('div');
      card.className = 'idea-card';
      const badgeHtml = idea.gap
        ? `<span class="idea-gap-badge">${escapeHtml(idea.gap)}</span>`
        : '<span class="badge">Gap Idea</span>';

      card.innerHTML = `
        <div class="idea-card-header">
          <span class="idea-title">${escapeHtml(idea.title)}</span>
          ${badgeHtml}
        </div>
        <div class="idea-body">${escapeHtml(idea.body)}</div>
        <div class="idea-actions-col">
          <div class="idea-actions-row">
            <button class="btn btn-success btn-xs btn-adopt-idea" data-idx="${idx}">✔ Adopt as Intent</button>
            <button class="btn btn-secondary btn-xs btn-edit-idea" data-idx="${idx}">✏ Edit in Bar</button>
          </div>
          <div class="idea-actions-row">
            <button class="btn btn-secondary btn-xs btn-scope-idea" data-idx="${idx}" title="Mark as out of scope for this domain">🚫 Out of Scope</button>
            <button class="btn btn-secondary btn-xs btn-pivot-idea" data-idx="${idx}" title="Explore alternative mechanisms for this gap">🔀 Different Angle</button>
            <button class="btn btn-secondary btn-xs btn-dismiss-idea" data-idx="${idx}" title="Dismiss idea">✕ Dismiss</button>
          </div>
        </div>
      `;

      const removeCardAndMaybeRefresh = () => {
        card.remove();
        const remainingCards = ideasDeck.querySelectorAll('.idea-card');
        if (remainingCards.length === 0 && autoProposeToggle.checked && currentSession && currentSession.state === 'open') {
          requestInspiration('', { isExplicitTopic: false });
        }
      };

      card.querySelector('.btn-adopt-idea').addEventListener('click', async () => {
        recordExcludedIdea(idea.rawTitle || idea.title);
        const intentText = `I want ${idea.title.toLowerCase()}: ${idea.body}`;
        const remainingCards = ideasDeck.querySelectorAll('.idea-card');
        // Keep remaining cards in the deck unless this was the last card
        if (remainingCards.length > 1) {
          skipNextAutoBrainstorm = true;
        }
        card.remove();
        await submitStakeholderTurn(intentText, 'typed');
        showToast('Idea adopted into transcript!', 'success');
      });

      card.querySelector('.btn-edit-idea').addEventListener('click', () => {
        recordExcludedIdea(idea.rawTitle || idea.title);
        const intentText = `I want ${idea.title.toLowerCase()}: ${idea.body}`;
        draftBox.value = intentText;
        draftBox.focus();
        btnSubmitTurn.disabled = false;
      });

      card.querySelector('.btn-scope-idea').addEventListener('click', async () => {
        recordExcludedIdea(idea.rawTitle || idea.title);
        const boundaryText = `Out of scope: ${idea.title} is not considered for this project.`;
        const remainingCards = ideasDeck.querySelectorAll('.idea-card');
        if (remainingCards.length > 1) {
          skipNextAutoBrainstorm = true;
        }
        card.remove();
        await submitStakeholderTurn(boundaryText, 'typed');
        showToast('Boundary recorded: Out of Scope', 'success');
      });

      card.querySelector('.btn-pivot-idea').addEventListener('click', () => {
        recordExcludedIdea(idea.rawTitle || idea.title);
        removeCardAndMaybeRefresh();
        showToast(`Exploring different angle for ${idea.title}...`, 'info');
        requestInspiration(`Alternative mechanisms for ${idea.title} (do NOT suggest ${idea.title.toLowerCase()})`, { isExplicitTopic: true });
      });

      card.querySelector('.btn-dismiss-idea').addEventListener('click', () => {
        recordExcludedIdea(idea.rawTitle || idea.title);
        removeCardAndMaybeRefresh();
      });

      ideasDeck.appendChild(card);
    });
  }

  function renderSocraticPrompt(data) {
    if (!proposalBox) {
      renderOpeningQuestion(data.text);
      return;
    }
    proposalBox.classList.remove('hidden');
    let displayText = data.text;
    let category = 'Socratic Prompt';
    const tagMatch = data.text.match(/^\[([^\]]+)\]\s*([\s\S]+)$/);
    if (tagMatch) {
      category = tagMatch[1].trim();
      displayText = tagMatch[2].trim();
    }
    if (proposalText) proposalText.textContent = displayText;
    if (proposalStatus) proposalStatus.textContent = category;
    if (btnAnswerProposal) btnAnswerProposal.disabled = false;
    if (btnAcceptProposal) btnAcceptProposal.disabled = false;
    if (btnEditProposal) btnEditProposal.disabled = false;
    if (btnRegenProposal) btnRegenProposal.disabled = false;
    if (btnRejectProposal) btnRejectProposal.disabled = false;
    if (proposalLintBadges) proposalLintBadges.innerHTML = '';

    if (data.flags && data.flags.length > 0 && proposalLintBadges) {
      data.flags.forEach((flag) => {
        const badge = document.createElement('span');
        badge.className = 'lint-badge';
        const label = flag.category.charAt(0).toUpperCase() + flag.category.slice(1);
        badge.textContent = `⚠️ ${label}: ${flag.terms.join(', ')}`;
        badge.title = flag.message;
        proposalLintBadges.appendChild(badge);
      });
    }
  }

  function hideProposal() {
    if (proposalBox) proposalBox.classList.add('hidden');
    currentProposalId = null;
    currentProposalText = null;
  }

  function showFacilitatorError(errMsg) {
    if (proposalBox) proposalBox.classList.add('hidden');
    if (facilitatorErrorBox) facilitatorErrorBox.classList.remove('hidden');
    if (facilitatorErrorMsg) facilitatorErrorMsg.textContent = errMsg;
  }

  function clearDraft() {
    draftBox.value = '';
    draftHasAsr = false;
    draftBox.placeholder = 'Type intent statement or hold mic to dictate... (Enter to send, Shift+Enter for space)';
    if (composerRole) composerRole.value = 'stakeholder';
    updateDraftModeUI();
  }

  function updateDraftModeUI() {
    const text = draftBox.value.trim();
    btnSubmitTurn.disabled = !currentSession || currentSession.state === 'closed' || text.length === 0;
    draftModeBadge.textContent = draftHasAsr ? 'Mode: Spoken (Dictated)' : 'Mode: Typed';
  }

  // Event Listeners - Sidecar Mode Tabs (if present)
  if (tabInspiration) {
    tabInspiration.addEventListener('click', () => {
      activeSidecarTab = 'inspiration';
      tabInspiration.classList.add('active');
      if (tabSocratic) tabSocratic.classList.remove('active');
      if (secInspiration) secInspiration.classList.remove('hidden');
      if (secSocratic) secSocratic.classList.add('hidden');
    });
  }

  if (tabSocratic) {
    tabSocratic.addEventListener('click', () => {
      activeSidecarTab = 'socratic';
      tabSocratic.classList.add('active');
      if (tabInspiration) tabInspiration.classList.remove('active');
      if (secSocratic) secSocratic.classList.remove('hidden');
      if (secInspiration) secInspiration.classList.add('hidden');
    });
  }

  if (btnCancelInlineInsert) {
    btnCancelInlineInsert.addEventListener('click', () => {
      cancelInlineInsert();
    });
  }

  // Event Listeners - Inspiration Topic Inputs
  topicInput.addEventListener('input', () => {
    updateTopicControlsUI();
  });

  if (btnClearTopic) {
    btnClearTopic.addEventListener('click', () => {
      topicInput.value = '';
      updateTopicControlsUI();
      requestInspiration('', { isExplicitTopic: false });
    });
  }

  btnBrainstorm.addEventListener('click', () => {
    const topicVal = topicInput.value.trim();
    requestInspiration(topicVal, { isExplicitTopic: topicVal.length > 0 });
  });

  topicInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      const topicVal = topicInput.value.trim();
      requestInspiration(topicVal, { isExplicitTopic: topicVal.length > 0 });
    } else if (e.key === 'Escape') {
      topicInput.value = '';
      updateTopicControlsUI();
    }
  });

  // Topic Mic Button
  micTopicBtn.addEventListener('mousedown', (e) => {
    e.preventDefault();
    startRecording('topic');
  });

  micTopicBtn.addEventListener('mouseup', (e) => {
    e.preventDefault();
    stopRecording();
  });

  // Main Mic Button
  micBtn.addEventListener('mousedown', (e) => {
    e.preventDefault();
    startRecording('main');
  });

  micBtn.addEventListener('mouseup', (e) => {
    e.preventDefault();
    stopRecording();
  });

  // Global Push-to-Talk Keyboard Shortcut (Ctrl+Alt+V)
  let pttKeyDown = false;
  window.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.altKey && (e.key === 'v' || e.key === 'V')) {
      if (!pttKeyDown && !isRecording && !micBtn.disabled) {
        pttKeyDown = true;
        e.preventDefault();
        startRecording('main');
      }
    }
  });

  window.addEventListener('keyup', (e) => {
    if (pttKeyDown && (e.key === 'v' || e.key === 'V' || e.key === 'Control' || e.key === 'Alt')) {
      pttKeyDown = false;
      stopRecording();
    }
  });

  // Chat Input Field Handling
  draftBox.addEventListener('input', () => {
    draftHasAsr = false;
    updateDraftModeUI();
  });

  draftBox.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submitCurrentTurn();
    } else if (e.key === 'Escape') {
      clearDraft();
    }
  });

  btnSubmitTurn.addEventListener('click', () => {
    submitCurrentTurn();
  });

  btnClearDraft.addEventListener('click', () => {
    clearDraft();
  });

  // Socratic Deck Actions (if present)
  if (btnAnswerProposal) {
    btnAnswerProposal.addEventListener('click', async () => {
      if (!currentProposalText) return;
      if (!currentSession || currentSession.state === 'closed') return;
      const text = currentProposalText;
      btnAnswerProposal.disabled = true;
      hideProposal();

      try {
        const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            role: 'facilitator',
            text,
            input_mode: 'typed',
          }),
        });
        if (!resp.ok) {
          const err = await resp.json().catch(() => ({}));
          throw new Error(err.message || `HTTP ${resp.status}`);
        }
        const turn = await resp.json();
        const turnLabel = turn && turn.number ? `[t${turn.number}]` : 'prompt';
        draftBox.focus();
        draftBox.placeholder = `Answering ${turnLabel}: "${text.slice(0, 45)}..."`;
        showToast(`Prompt ${turnLabel} added to transcript. Ready for answer.`, 'success');
      } catch (err) {
        showToast(`Failed to accept prompt: ${err.message}`);
      } finally {
        btnAnswerProposal.disabled = false;
      }
    });
  }

  if (btnRetryProposal) {
    btnRetryProposal.addEventListener('click', () => {
      facilitatorErrorBox.classList.add('hidden');
      if (activeSidecarTab === 'inspiration') {
        requestInspiration('');
      } else {
        requestFacilitatorProposal();
      }
    });
  }

  if (btnDismissFacilitatorError) {
    btnDismissFacilitatorError.addEventListener('click', () => {
      facilitatorErrorBox.classList.add('hidden');
    });
  }

  if (btnAcceptProposal) {
    btnAcceptProposal.addEventListener('click', async () => {
      if (!currentProposalText) return;
      const text = currentProposalText;
      hideProposal();

      try {
        const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/turns`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            role: 'facilitator',
            text,
            input_mode: 'typed',
          }),
        });
        if (!resp.ok) {
          const err = await resp.json().catch(() => ({}));
          throw new Error(err.message || `HTTP ${resp.status}`);
        }
      } catch (err) {
        showToast(`Failed to accept proposal: ${err.message}`);
      }
    });
  }

  if (btnEditProposal) {
    btnEditProposal.addEventListener('click', () => {
      if (!currentProposalText) return;
      draftBox.value = currentProposalText;
      if (composerRole) composerRole.value = 'facilitator';
      draftHasAsr = false;
      draftBox.placeholder = 'Edit facilitator prompt, then press Enter or Send to post...';
      updateDraftModeUI();
      hideProposal();
      draftBox.focus();
    });
  }

  if (btnRegenProposal) {
    btnRegenProposal.addEventListener('click', () => {
      requestFacilitatorProposal();
    });
  }

  if (btnRejectProposal) {
    btnRejectProposal.addEventListener('click', () => {
      hideProposal();
    });
  }

  btnAskOpening.addEventListener('click', () => {
    requestFacilitatorProposal();
    btnAskOpening.classList.add('hidden');
  });

  // Session Close, Reopen, Export & Delete
  btnCloseSession.addEventListener('click', async () => {
    if (!currentSession) return;
    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/close`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });

      if (!resp.ok) {
        const data = await resp.json().catch(() => ({}));
        if (data.errors && data.errors.length > 0) {
          const msgs = data.errors.map((e) => e.turn ? `[t${e.turn}] ${e.message}` : e.message).join('; ');
          throw new Error(`Validation failed: ${msgs}`);
        }
        throw new Error(data.message || `Close failed (${resp.status})`);
      }

      currentSession.state = 'closed';
      updateSessionStateUI();
      showToast('Session closed successfully. Ready for canonical export.', 'success');
    } catch (err) {
      showToast(err.message);
    }
  });

  btnReopenSession.addEventListener('click', async () => {
    if (!currentSession) return;
    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}/reopen`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });

      if (!resp.ok) {
        const data = await resp.json().catch(() => ({}));
        throw new Error(data.message || `Reopen failed (${resp.status})`);
      }

      const session = await resp.json();
      currentSession = session;
      updateSessionStateUI();
      showToast('Session reopened.', 'success');
    } catch (err) {
      showToast(err.message);
    }
  });

  btnExportTranscript.addEventListener('click', () => {
    if (!currentSession || currentSession.state !== 'closed') return;
    const url = `/api/v1/elicitation/sessions/${currentSession.elicitation_id}/transcript`;
    window.location.href = url;
  });

  btnDeleteSession.addEventListener('click', async () => {
    if (!currentSession) return;
    if (!confirm('Are you sure you want to discard this draft session? All in-memory turn text will be dropped.')) {
      return;
    }

    try {
      const resp = await fetch(`/api/v1/elicitation/sessions/${currentSession.elicitation_id}`, {
        method: 'DELETE',
      });
      if (!resp.ok && resp.status !== 404) {
        throw new Error(`Delete failed (${resp.status})`);
      }

      if (sseSource) {
        sseSource.close();
        sseSource = null;
      }
      currentSession = null;
      setupCard.classList.remove('hidden');
      sessionInfoCard.classList.add('hidden');
      gapQuestionsCard.classList.add('hidden');
      timelineTitle.textContent = 'No Active Session';
      turnsList.innerHTML = '<div class="empty-state">Create or start a session to begin intent elicitation.</div>';
      clearDraft();
      hideProposal();
      showToast('Draft session discarded.', 'success');
    } catch (err) {
      showToast(err.message);
    }
  });


  function escapeHtml(s) {
    const div = document.createElement('div');
    div.textContent = s;
    return div.innerHTML;
  }

  // Init
  initCapabilities();
})();
