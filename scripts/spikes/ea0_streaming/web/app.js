// EA-0.2: Client-side Interactive Streaming Dictation Logic

document.addEventListener('DOMContentLoaded', () => {
  const micBtn = document.getElementById('micBtn');
  const micLabel = document.getElementById('micLabel');
  const preview = document.getElementById('streamingPreview');
  const previewText = document.getElementById('previewText');
  const turnInput = document.getElementById('turnInput');
  const submitBtn = document.getElementById('submitBtn');
  const conversation = document.getElementById('conversation');

  let audioContext = null;
  let mediaStream = null;
  let processorNode = null;
  let isRecording = false;

  // Initialize Web Audio API
  async function startRecording() {
    if (isRecording) return;
    try {
      mediaStream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          sampleRate: 16000,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });

      audioContext = new (window.AudioContext || window.webkitAudioContext)({
        sampleRate: 16000,
      });

      const source = audioContext.createMediaStreamSource(mediaStream);
      // Use ScriptProcessor / AudioWorklet for 16kHz PCM capture
      processorNode = audioContext.createScriptProcessor(4096, 1, 1);

      processorNode.onaudioprocess = (e) => {
        if (!isRecording) return;
        const inputData = e.inputBuffer.getChannelData(0);
        // In full WASM integration, feed inputData Float32Array directly into worker.
        // For visual demo, simulate live partial token streaming:
        onAudioFrameCaptured(inputData);
      };

      source.connect(processorNode);
      processorNode.connect(audioContext.destination);

      isRecording = true;
      micBtn.classList.add('recording');
      micLabel.textContent = 'Listening... (Release to Stop)';
      preview.classList.add('active');
      previewText.innerHTML = '<span class="streaming-tokens">Listening to voice...</span>';
    } catch (err) {
      console.error('Microphone error:', err);
      alert('Microphone access denied or unsupported: ' + err.message);
    }
  }

  function stopRecording() {
    if (!isRecording) return;
    isRecording = false;

    if (processorNode) {
      processorNode.disconnect();
      processorNode = null;
    }
    if (mediaStream) {
      mediaStream.getTracks().forEach((t) => t.stop());
      mediaStream = null;
    }
    if (audioContext) {
      audioContext.close();
      audioContext = null;
    }

    micBtn.classList.remove('recording');
    micLabel.textContent = 'Push-to-Talk (Hold)';
    preview.classList.remove('active');

    // Transfer recognized text into the editable turn input
    const recognizedText = previewText.textContent.replace('Listening to voice...', '').trim();
    if (recognizedText) {
      turnInput.value = (turnInput.value ? turnInput.value + ' ' : '') + recognizedText;
      previewText.textContent = 'Hold Push-to-Talk or toggle mic to dictate...';
    }
  }

  // Simulated streaming recognition callback
  let simulatedTokens = [
    'We', 'need', 'a', 'lightweight', 'streaming', 'dictation', 'system',
    'that', 'runs', 'locally', 'in', 'the', 'browser'
  ];
  let tokenIdx = 0;
  let frameCounter = 0;

  function onAudioFrameCaptured(pcm) {
    frameCounter++;
    // Simulate real-time streaming tokens every ~250ms
    if (frameCounter % 3 === 0 && tokenIdx < simulatedTokens.length) {
      const partial = simulatedTokens.slice(0, tokenIdx + 1).join(' ');
      previewText.innerHTML = `<span class="streaming-tokens">${partial}</span>`;
      tokenIdx++;
    }
  }

  // Mouse / Touch Push-to-Talk events
  micBtn.addEventListener('mousedown', startRecording);
  window.addEventListener('mouseup', () => {
    if (isRecording) stopRecording();
  });

  micBtn.addEventListener('touchstart', (e) => {
    e.preventDefault();
    startRecording();
  });
  window.addEventListener('touchend', () => {
    if (isRecording) stopRecording();
  });

  // Global Keyboard shortcuts: Ctrl+Alt+V or Spacebar
  window.addEventListener('keydown', (e) => {
    // Ctrl+Alt+V push-to-talk
    if (e.ctrlKey && e.altKey && e.key.toLowerCase() === 'v') {
      e.preventDefault();
      if (!isRecording) startRecording();
    }
  });

  window.addEventListener('keyup', (e) => {
    if (e.key.toLowerCase() === 'v' && isRecording) {
      stopRecording();
    }
  });

  // Turn submission
  submitBtn.addEventListener('click', submitTurn);
  turnInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submitTurn();
    }
  });

  function submitTurn() {
    const text = turnInput.value.trim();
    if (!text) return;

    // Append stakeholder turn to conversation panel
    const stakeholderCard = document.createElement('div');
    stakeholderCard.className = 'turn-card turn-stakeholder';
    stakeholderCard.innerHTML = `
      <div class="turn-author">Stakeholder (You)</div>
      <p>${escapeHtml(text)}</p>
    `;
    conversation.appendChild(stakeholderCard);
    turnInput.value = '';
    tokenIdx = 0;
    frameCounter = 0;
    conversation.scrollTop = conversation.scrollHeight;

    // Send text turn to tks serve (/api/v1/elicitation/{id}/turns)
    console.log('[TKS Elicitation] Squirting UTF-8 text turn to server:', text);

    // Simulate Socratic Facilitator response
    setTimeout(() => {
      const facilitatorCard = document.createElement('div');
      facilitatorCard.className = 'turn-card turn-facilitator';
      facilitatorCard.innerHTML = `
        <div class="turn-author">Facilitator (Socratic Prompt)</div>
        <p>What are the key constraints or boundaries for this capability? For instance, what should it explicitly *not* do?</p>
      `;
      conversation.appendChild(facilitatorCard);
      conversation.scrollTop = conversation.scrollHeight;
    }, 600);
  }

  function escapeHtml(str) {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  }
});
