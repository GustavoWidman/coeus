/** What this device is, as the screens speak of it. */

/** an iPad asking for the desktop site says it is a Mac, but a Mac has no touch screen */
const ipad = (ua: string, touch: number) => /iPad/.test(ua) || (/Macintosh/.test(ua) && touch > 1);

/** what to call this device in a sentence: "This iPhone can open coeus…" */
export function noun(ua = navigator.userAgent, touch = navigator.maxTouchPoints ?? 0): string {
	if (/iPhone/.test(ua)) return 'iPhone';
	if (ipad(ua, touch)) return 'iPad';
	if (/Android/.test(ua)) return 'phone';
	if (/Macintosh/.test(ua)) return 'Mac';
	return 'computer';
}

/** an iPhone or iPad, where only the Home Screen app gets notifications */
export const ios = (ua = navigator.userAgent, touch = navigator.maxTouchPoints ?? 0) =>
	/iPhone/.test(ua) || ipad(ua, touch);

/** running as a Home Screen app rather than in a browser tab */
export const standalone = () =>
	matchMedia('(display-mode: standalone)').matches ||
	(navigator as { standalone?: boolean }).standalone === true;
