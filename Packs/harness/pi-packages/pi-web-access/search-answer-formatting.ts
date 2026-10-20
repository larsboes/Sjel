interface AnswerResult {
	title: string;
	url: string;
	snippet: string;
}

/** Format each snippet with a clickable source, preserving the provider's URL. */
export function formatSearchResultsAsAnswer(results: readonly AnswerResult[]): string {
	return results.map((result) => {
		const title = (result.title.trim() || result.url).replace(/[\\[\]]/g, "\\$&");
		const url = result.url.replace(/</g, "%3C").replace(/>/g, "%3E");
		const source = `Source: [${title}](<${url}>)`;
		return result.snippet ? `${result.snippet}\n${source}` : source;
	}).join("\n\n");
}
