export const meta = {
  name: 'review-branch',
  description:
    'Review the branch with the implementation-reviewer passes, website-reviewer and test-reviewer in parallel, then merge their findings in one reviewer summary',
  whenToUse:
    'Before opening a pull request, from the submit-pr skill, or when the user asks for a full review of the branch. Optional args: the files or commits to review.',
  phases: [
    {
      title: 'Review',
      detail: 'six implementation-reviewer categories, website-reviewer and test-reviewer, in parallel',
    },
    { title: 'Summarize', detail: 'reviewer merges the findings with its own review' },
  ],
}

const CATEGORIES = ['ownership', 'safety', 'errors', 'fallbacks', 'async', 'ladder']

const FINDINGS_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      description: 'Findings from most to least severe; empty when there are none',
      items: {
        type: 'object',
        properties: {
          file: { type: 'string' },
          line: { type: 'integer' },
          category: { type: 'string' },
          check: { type: 'integer', description: 'Check number from the agent definition' },
          summary: { type: 'string', description: 'What is wrong' },
          failureScenario: { type: 'string', description: 'A concrete scenario where it causes a problem' },
          suggestedFix: { type: 'string' },
          uncertain: { type: 'boolean', description: 'True when the finding could not be confirmed' },
        },
        required: ['file', 'line', 'category', 'check', 'summary', 'failureScenario', 'suggestedFix', 'uncertain'],
      },
    },
    verdict: {
      type: 'string',
      description:
        'One line: ready, ready after the listed fixes, needs rework, or no Rust changes / no website changes / no test changes',
    },
  },
  required: ['findings', 'verdict'],
}

const SUMMARY_SCHEMA = {
  type: 'object',
  properties: {
    report: {
      type: 'string',
      description: 'Markdown report: findings from most to least severe, duplicates removed, passes not reviewed named',
    },
    verdict: { type: 'string', enum: ['ready', 'ready after the listed fixes', 'needs rework'] },
  },
  required: ['report', 'verdict'],
}

const target = Array.isArray(args) ? args.join(', ') : args
const scope = target
  ? `Review these files or commits instead of the branch diff: ${target}.`
  : 'Review the branch diff: `git diff main...HEAD` plus `git diff HEAD`.'

const passes = [
  ...CATEGORIES.map((category) => ({
    name: `implementation-reviewer:${category}`,
    agentType: 'implementation-reviewer',
    prompt: `Run only the \`${category}\` category. ${scope} Return every finding with its category and check number.`,
  })),
  {
    name: 'website-reviewer',
    agentType: 'website-reviewer',
    prompt: `${scope} Use category \`website\` and your own check numbers for the findings.`,
  },
  {
    name: 'test-reviewer',
    agentType: 'test-reviewer',
    prompt: `${scope} Use category \`tests\` and your own check numbers for the findings.`,
  },
]

phase('Review')
const results = await parallel(
  passes.map((pass) => () =>
    agent(pass.prompt, {
      label: pass.name,
      phase: 'Review',
      agentType: pass.agentType,
      schema: FINDINGS_SCHEMA,
    }),
  ),
)

const reviewed = passes
  .map((pass, i) => ({ pass: pass.name, result: results[i] }))
  .filter((r) => r.result)
  .map((r) => ({ pass: r.pass, verdict: r.result.verdict, findings: r.result.findings }))
const notReviewed = passes.filter((_, i) => !results[i]).map((pass) => pass.name)
if (notReviewed.length) log(`Not reviewed: ${notReviewed.join(', ')}`)

phase('Summarize')
const summary = await agent(
  [
    `${scope}`,
    'Do your own general review, then merge these findings from the implementation-reviewer, website-reviewer and test-reviewer passes with yours.',
    'Remove duplicates, rank everything from most to least severe, and give one overall verdict.',
    notReviewed.length
      ? `These passes returned no result and count as not reviewed: ${notReviewed.join(', ')}. Name them under "Not reviewed" in the report. The verdict can't be "ready".`
      : 'Every pass returned a result.',
    `Findings per pass (JSON):\n${JSON.stringify(reviewed, null, 2)}`,
  ].join('\n\n'),
  { label: 'reviewer:summary', phase: 'Summarize', agentType: 'reviewer', schema: SUMMARY_SCHEMA },
)

if (!summary) {
  return { verdict: 'not reviewed', notReviewed: [...notReviewed, 'reviewer:summary'], passes: reviewed }
}
// A missing pass must never read as a clean pass.
const verdict = notReviewed.length && summary.verdict === 'ready' ? 'ready after the listed fixes' : summary.verdict
return { verdict, notReviewed, report: summary.report, passes: reviewed }
