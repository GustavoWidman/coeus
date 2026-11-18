/**
 * Prints the fallback faces in src/lib/ui/fonts.css: a local font scaled and given the web
 * font's vertical metrics, so text keeps its width and line boxes when the web font swaps in.
 *
 *   bun scripts/fonts.ts            (from web/)
 */
import { createFontStack } from '@capsizecss/core';
import arial from '@capsizecss/metrics/arial';
import courierNew from '@capsizecss/metrics/courierNew';
import geistMono from '@capsizecss/metrics/geistMono';
import instrumentSans from '@capsizecss/metrics/instrumentSans';

for (const stack of [
	[instrumentSans, arial],
	[geistMono, courierNew]
]) {
	console.log(createFontStack(stack).fontFaces);
}
