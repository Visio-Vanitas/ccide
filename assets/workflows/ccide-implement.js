export const meta = {
  name: 'ccide-implement',
  description: 'Scout non-overlapping file batches, implement in parallel, and verify with reviewer',
  whenToUse: 'Break an implementation task into disjoint file batches, execute parallel coder workers, and run reviewer verification',
  phases: [
    { title: 'Scout', detail: 'Map non-overlapping file batches' },
    { title: 'Implement', detail: 'Parallel coder workers per batch' },
    { title: 'Verify', detail: 'Reviewer verification pass' },
  ],
};

const PLAN_SCHEMA = {
  type: 'object',
  properties: {
    batches: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          name: { type: 'string' },
          files: { type: 'array', items: { type: 'string' } },
          instructions: { type: 'string' },
        },
        required: ['name', 'files', 'instructions'],
      },
    },
  },
  required: ['batches'],
};

const VERIFY_SCHEMA = {
  type: 'object',
  properties: {
    passed: { type: 'boolean' },
    summary: { type: 'string' },
    issues: { type: 'array', items: { type: 'string' } },
  },
  required: ['passed', 'summary'],
};

const task = args?.task || args?.prompt || (typeof args === 'string' ? args : 'Implement requested task');

phase('Scout');
const plan = await agent(
  `Analyze the codebase and partition the following task into non-overlapping file batches so parallel workers never touch the same file.\nTask: ${task}`,
  { phase: 'Scout', label: 'scout-batches', model: '@scout', effort: '@scout', schema: PLAN_SCHEMA },
);

const batches = plan?.batches?.length
  ? plan.batches
  : [{ name: 'main', files: [], instructions: task }];
log(`Planned ${batches.length} non-overlapping batch(es)`);

phase('Implement');
const implementations = await parallel(
  batches.map((batch, idx) => () =>
    agent(
      `Implement batch "${batch.name}".\nTarget files: ${(batch.files || []).join(', ') || '(determine as needed)'}\nInstructions: ${batch.instructions}\nOverall task: ${task}`,
      { phase: 'Implement', label: batch.name || `batch-${idx}`, model: '@coder', effort: '@coder' },
    ),
  ),
);

phase('Verify');
const implSummary = implementations
  .filter(Boolean)
  .map((out, idx) => `[Batch ${idx}] ${String(out).slice(0, 1200)}`)
  .join('\n\n');

const verification = await agent(
  `Verify the implementation for task: ${task}\n\nWorker outputs:\n${implSummary}\n\nCheck for regressions, missing edge cases, and build/test health.`,
  { phase: 'Verify', label: 'verify-impl', model: '@reviewer', effort: '@reviewer', schema: VERIFY_SCHEMA },
);

return {
  task,
  batches: batches.length,
  implementations: implementations.filter(Boolean),
  verification,
};
