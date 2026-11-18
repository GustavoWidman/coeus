const query = matchMedia('(prefers-reduced-motion: reduce)');

/** whether to skip every animation the scripts start; the stylesheet shortens its own */
export const reduced = () => query.matches;
