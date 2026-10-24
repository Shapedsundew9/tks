// EA-0.2: Reproducible WER / correction-effort scoring matching scripts/spikes/ea0_asr/wer.rs exactly.

const FILLERS = new Set(["hmm", "mm", "mhm", "mmm", "uh", "um", "er", "ah"]);

const STANDARDS = {
  "mr": "mister",
  "mrs": "missus",
  "dr": "doctor",
  "ok": "okay",
  "gonna": "going to",
  "wanna": "want to",
};

const ONES = [
  "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
  "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
  "seventeen", "eighteen", "nineteen"
];

const TENS = [
  "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"
];

function numberWords(n) {
  if (n <= 19) {
    return ONES[n];
  } else if (n <= 99) {
    const t = TENS[Math.floor(n / 10)];
    return (n % 10 === 0) ? t : `${t} ${ONES[n % 10]}`;
  } else if (n <= 999) {
    const rest = n % 100;
    const head = `${ONES[Math.floor(n / 100)]} hundred`;
    return (rest === 0) ? head : `${head} ${numberWords(rest)}`;
  } else {
    const rest = n % 1000;
    const head = `${numberWords(Math.floor(n / 1000))} thousand`;
    return (rest === 0) ? head : `${head} ${numberWords(rest)}`;
  }
}

function normalize(text) {
  if (!text) return [];
  const cleaned = text.toLowerCase().replace(/[^a-z0-9']/g, ' ');
  const rawWords = cleaned.trim().split(/\s+/).filter(Boolean);
  const out = [];

  for (const raw of rawWords) {
    const word = raw.replace(/^'+|'+$/g, '');
    if (!word || FILLERS.has(word)) {
      continue;
    }
    if (STANDARDS[word]) {
      for (const w of STANDARDS[word].split(' ')) {
        out.push(w);
      }
    } else if (/^\d+$/.test(word)) {
      const n = parseInt(word, 10);
      if (n < 10000) {
        for (const w of numberWords(n).split(' ')) {
          out.push(w);
        }
      } else {
        out.push(word);
      }
    } else {
      out.push(word);
    }
  }
  return out;
}

function score(reference, hypothesis) {
  const r = normalize(reference);
  const h = normalize(hypothesis);

  // dp[i][j] = [cost, subs, dels, ins]
  const dp = Array.from({ length: r.len + 1 || r.length + 1 }, () =>
    Array.from({ length: h.length + 1 }, () => [0, 0, 0, 0])
  );

  for (let i = 1; i <= r.length; i++) {
    dp[i][0] = [i, 0, i, 0];
  }
  for (let j = 1; j <= h.length; j++) {
    dp[0][j] = [j, 0, 0, j];
  }

  for (let i = 1; i <= r.length; i++) {
    for (let j = 1; j <= h.length; j++) {
      const diag = dp[i - 1][j - 1];
      const same = r[i - 1] === h[j - 1];
      const sub = [
        diag[0] + (same ? 0 : 1),
        diag[1] + (same ? 0 : 1),
        diag[2],
        diag[3],
      ];
      const up = dp[i - 1][j];
      const del = [up[0] + 1, up[1], up[2] + 1, up[3]];
      const left = dp[i][j - 1];
      const ins = [left[0] + 1, left[1], left[2], left[3] + 1];

      // pick minimum cost; tie break: sub > del > ins
      let best = sub;
      if (del[0] < best[0]) best = del;
      if (ins[0] < best[0]) best = ins;
      dp[i][j] = best;
    }
  }

  const [_, substitutions, deletions, insertions] = dp[r.length][h.length];
  return {
    substitutions,
    deletions,
    insertions,
    total: substitutions + deletions + insertions,
    reference_words: r.length,
    wer: r.length === 0 ? (insertions === 0 ? 0.0 : 1.0) : (substitutions + deletions + insertions) / r.length,
  };
}

module.exports = {
  normalize,
  score,
  numberWords,
};
