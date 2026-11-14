export const meta = {
  name: 'ccide-review',
  description: 'Multi-dimension code review with adversarial multi-model judge panel',
  whenToUse: 'Review code changes or target paths across correctness, security, performance, and concurrency with multi-model verification',
  phases: [
    { title: 'Scout', detail: 'Discover review targets and subsystems' },
    { title: 'Find', detail: 'Parallel dimension finders across targets' },
    { title: 'Verify', detail: 'Adversarial multi-model judge panel' },
  ],
};

const TARGETS_SCHEMA = {
  type: 'object',
  properties: {
    targets: {
      type: 'array',
      items: { type: 'string' },
    },
  },
  required: ['targets'],
};

const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          file: { type: 'string' },
          line: { type: 'number' },
          severity: { type: 'string' },
          title: { type: 'string' },
          description: { type: 'string' },
        },
        required: ['file', 'line', 'severity', 'title', 'description'],
      },
    },
  },
  required: ['findings'],
};

const VERDICT_SCHEMA = {
  type: 'object',
  properties: {
    confirmed: { type: 'boolean' },
    reason: { type: 'string' },
    severity: { type: 'string' },
  },
  required: ['confirmed', 'reason'],
};

phase('Scout');
let targets = Array.isArray(args?.targets) && args.targets.length > 0 ? args.targets : null;
if (!targets) {
  const scopeHint = args?.prompt ? `Focus on: ${args.prompt}` : 'Inspect modified or core repository subsystems.';
  const scouted = await agent(
    `List the key file paths or subsystem directories to review. ${scopeHint}`,
    { phase: 'Scout', label: 'scout-targets', model: '@scout', effort: '@scout', schema: TARGETS_SCHEMA },
  );
  targets = scouted?.targets?.length ? scouted.targets : ['.'];
}
log(`Scouting complete: ${targets.length} target(s)`);

phase('Find');
const DIMENSIONS = [
  { id: 'correctness', focus: 'logic errors, edge cases, state invariants, off-by-one bugs' },
  { id: 'security', focus: 'injection, path traversal, auth bypass, unsafe deserialization' },
  { id: 'performance', focus: 'unbounded allocations, N+1 loops, blocking hot paths' },
  { id: 'concurrency', focus: 'race conditions, deadlocks, stale cache or file-lock issues' },
];

const targetList = targets.join(', ');
const rawFindings = await parallel(
  DIMENSIONS.map((dim) => () =>
    agent(
      `Review [${targetList}] strictly for ${dim.id} issues (${dim.focus}). Report only concrete, verifiable defects with exact file and line.`,
      { phase: 'Find', label: `find-${dim.id}`, model: '@scout', effort: '@scout', schema: FINDINGS_SCHEMA },
    ),
  ),
);

const seen = new Set();
const candidates = [];
for (const batch of rawFindings.filter(Boolean)) {
  for (const f of batch.findings || []) {
    const key = `${f.file}:${f.line}:${f.title}`;
    if (!seen.has(key)) {
      seen.add(key);
      candidates.push(f);
    }
  }
}
log(`Found ${candidates.length} unique candidate finding(s)`);

if (candidates.length === 0) {
  return { confirmed: [], total_candidates: 0 };
}

phase('Verify');
const pool = args?.__ccide?.roles?.reviewer?.pool;
const judges = Array.isArray(pool) && pool.length > 0 ? pool : ['@reviewer'];
const majority = Math.ceil(judges.length / 2);

const verdicts = await parallel(
  candidates.map((cand, idx) => () =>
    parallel(
      judges.map((judgeModel, jIdx) => () =>
        agent(
          `Adversarially verify this candidate finding. Try to REFUTE it by checking the actual code; set confirmed=false if uncertain.\n` +
            `File: ${cand.file}:${cand.line}\nSeverity: ${cand.severity}\nTitle: ${cand.title}\nDetail: ${cand.description}`,
          { phase: 'Verify', label: `judge-${idx}-${jIdx}`, model: judgeModel, effort: '@reviewer', schema: VERDICT_SCHEMA },
        ),
      ),
    ).then((votes) => {
      const valid = votes.filter(Boolean);
      const yesCount = valid.filter((v) => v.confirmed).length;
      return {
        ...cand,
        confirmed: yesCount >= majority,
        votes: yesCount,
        total_judges: judges.length,
        reasons: valid.map((v) => v.reason),
      };
    }),
  ),
);

const confirmed = verdicts.filter(Boolean).filter((v) => v.confirmed);
return {
  confirmed,
  total_candidates: candidates.length,
};
