export const meta = {
  name: 'ccide-consensus',
  description: 'Run prompt concurrently across multiple models and synthesize the best answer',
  whenToUse: 'Compare independent answers from multiple models in parallel and synthesize a unified result',
  phases: [
    { title: 'Generate', detail: 'Concurrent candidate generation across models' },
    { title: 'Synthesize', detail: 'Plan model consensus synthesis' },
  ],
};

const prompt = args?.prompt || (typeof args === 'string' ? args : '');
const pool = args?.__ccide?.roles?.reviewer?.pool;
const models = Array.isArray(args?.models) && args.models.length > 0
  ? args.models
  : (Array.isArray(pool) && pool.length > 0 ? pool : ['@coder', '@reviewer']);

phase('Generate');
const rawOutputs = await parallel(
  models.map((m, idx) => () =>
    agent(prompt, { phase: 'Generate', label: `candidate-${idx}-${m}`, model: m }),
  ),
);

const candidates = models
  .map((m, idx) => ({ model: m, response: rawOutputs[idx] }))
  .filter((c) => c.response !== null && c.response !== undefined);

log(`Collected ${candidates.length}/${models.length} candidate response(s)`);

phase('Synthesize');
const formatted = candidates
  .map((c, idx) => `### Candidate ${idx + 1} (${c.model})\n${typeof c.response === 'string' ? c.response : JSON.stringify(c.response)}`)
  .join('\n\n');

const synthesis = await agent(
  `Synthesize the strongest, most accurate unified answer from these independent candidate responses to the prompt.\n\nPrompt:\n${prompt}\n\n${formatted}`,
  { phase: 'Synthesize', label: 'synthesize', model: '@plan', effort: '@plan' },
);

return {
  candidates,
  synthesis,
};
