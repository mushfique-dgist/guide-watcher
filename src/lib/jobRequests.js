function phase(provider, model, effort) {
  return { provider, model, effort };
}

export const PREP_PROVIDER = 'codex-chatgpt';
export const WRITER_PROVIDER = 'claude-code';

function fixedFallbacks(fallbacks) {
  return fallbacks.map((item, index) => phase(
    index === fallbacks.length - 1 ? PREP_PROVIDER : WRITER_PROVIDER,
    item.model,
    item.effort,
  ));
}

function collectionFallbacks(fallbacks) {
  return fallbacks.map(item => phase(PREP_PROVIDER, item.model, item.effort));
}

export function buildGenerateOptions(input) {
  return {
    prep: phase(PREP_PROVIDER, input.prepModel, input.prepEffort),
    collectionFallbacks: collectionFallbacks(input.collectionFallbacks),
    writer: phase(WRITER_PROVIDER, input.writerModel, input.writerEffort),
    fallbacks: fixedFallbacks(input.fallbacks),
    courseProfile: input.courseProfile,
    requireSlideCoverage: input.requireSlideCoverage,
    requireVisuals: input.requireVisuals,
  };
}

export function buildResumePrepOptions(input) {
  return {
    prep: phase(PREP_PROVIDER, input.prepModel, input.prepEffort),
    collectionFallbacks: collectionFallbacks(input.collectionFallbacks),
    writer: phase(WRITER_PROVIDER, input.writerModel, input.writerEffort),
    fallbacks: fixedFallbacks(input.fallbacks),
  };
}
