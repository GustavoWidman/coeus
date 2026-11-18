// Sits on the dashboard's route. Cloudflare answers 530 when no tunnel connector is up, so on a
// 530, or when the origin cannot be reached at all, this answers with coeus's own offline page.
// Everything else passes through as it came, streams included.

const PAGE = __PAGE__;
const CSP = __CSP__;

export default {
	async fetch(request) {
		let response;
		try {
			response = await fetch(request);
		} catch {
			return offline(request);
		}
		return response.status === 530 ? offline(request) : response;
	}
};

function offline(request) {
	const headers = {
		'cache-control': 'no-store',
		'retry-after': '15',
		'coeus-offline': '1',
		'x-content-type-options': 'nosniff'
	};
	const page = isPage(request);
	if (page) {
		headers['content-type'] = 'text/html; charset=utf-8';
		headers['content-security-policy'] = CSP;
		headers['referrer-policy'] = 'no-referrer';
	} else {
		headers['content-type'] = 'application/json';
	}
	const body = request.method === 'HEAD' ? null : page ? PAGE : '{"offline":true}';
	return new Response(body, { status: 503, headers });
}

// Safari before 16.4 sends no Sec-Fetch-Mode, so a request without one that accepts HTML counts.
function isPage(request) {
	if (request.method !== 'GET' && request.method !== 'HEAD') return false;
	const mode = request.headers.get('sec-fetch-mode');
	if (mode) return mode === 'navigate';
	return (request.headers.get('accept') ?? '').includes('text/html');
}
