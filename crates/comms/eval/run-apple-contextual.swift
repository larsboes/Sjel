import Darwin
import Foundation
import NaturalLanguage

struct Acceptance: Decodable {
    let minTop1Relevant: Double
    let minPairwiseAccuracy: Double
    let minMeanNdcg: Double
}

struct Candidate: Decodable {
    let id: String
    let language: String
    let text: String
    let relevance: Int
}

struct Query: Decodable {
    let id: String
    let language: String
    let text: String
    let candidates: [Candidate]
}

struct Corpus: Decodable {
    let schemaVersion: Int
    let acceptance: Acceptance
    let queries: [Query]
}

struct RankedCandidate {
    let candidate: Candidate
    let score: Double
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("apple contextual relevance eval: \(message)\n".utf8))
    exit(1)
}

func nlLanguage(for language: String) -> NLLanguage {
    switch language {
    case "de":
        return .german
    case "en":
        return .english
    default:
        fail("unsupported language \(language)")
    }
}

func cosine(_ left: [Double], _ right: [Double]) -> Double {
    guard !left.isEmpty, left.count == right.count else { return 0 }
    var dot = 0.0
    var leftNorm = 0.0
    var rightNorm = 0.0
    for index in left.indices {
        dot += left[index] * right[index]
        leftNorm += left[index] * left[index]
        rightNorm += right[index] * right[index]
    }
    return dot / sqrt(leftNorm * rightNorm)
}

func dcg(_ judgements: [Int]) -> Double {
    judgements.enumerated().reduce(0) { total, item in
        let (index, judgement) = item
        return total + (pow(2.0, Double(judgement)) - 1.0) / log2(Double(index + 2))
    }
}

func meanPooledVector(
    model: NLContextualEmbedding,
    text: String,
    language: String
) -> [Double] {
    let result: NLContextualEmbeddingResult
    do {
        result = try model.embeddingResult(
            for: text,
            language: nlLanguage(for: language)
        )
    } catch {
        fail("cannot embed \(language) text: \(error)")
    }

    var sum = [Double](repeating: 0, count: model.dimension)
    var count = 0
    result.enumerateTokenVectors(in: text.startIndex..<text.endIndex) { vector, _ in
        guard vector.count == sum.count else {
            fail("unexpected token-vector dimension \(vector.count)")
        }
        for index in vector.indices {
            sum[index] += vector[index]
        }
        count += 1
        return true
    }
    guard count > 0 else {
        fail("model produced no token vectors for \(language) text")
    }
    return sum.map { $0 / Double(count) }
}

let arguments = Array(CommandLine.arguments.dropFirst())
let requestAssets = arguments.contains("--request-assets")
let defaultCorpus = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("relevance-corpus.json")
let corpusURL = arguments.first(where: { !$0.hasPrefix("--") })
    .map { URL(fileURLWithPath: $0) } ?? defaultCorpus
let decoder = JSONDecoder()
decoder.keyDecodingStrategy = .convertFromSnakeCase

let corpus: Corpus
do {
    corpus = try decoder.decode(Corpus.self, from: Data(contentsOf: corpusURL))
} catch {
    fail("cannot read corpus: \(error)")
}
guard corpus.schemaVersion == 1, !corpus.queries.isEmpty else {
    fail("expected schema_version 1 with queries")
}

guard let model = NLContextualEmbedding(script: .latin) else {
    fail("no Latin contextual embedding is registered")
}

print("Comms DE/EN relevance eval · Apple NLContextualEmbedding")
print(
    "  model=\(model.modelIdentifier) revision=\(model.revision) dimensions=\(model.dimension) max-sequence=\(model.maximumSequenceLength)"
)
print(
    "  languages=\(model.languages.map(\.rawValue).sorted().joined(separator: ",")) assets=\(model.hasAvailableAssets ? "available" : "unavailable")"
)
guard model.languages.contains(.german), model.languages.contains(.english) else {
    fail("Latin model does not declare both German and English")
}
if !model.hasAvailableAssets, requestAssets {
    print("  requesting Apple-managed Latin model assets...")
    let semaphore = DispatchSemaphore(value: 0)
    var assetResult: NLContextualEmbedding.AssetsResult?
    var assetError: Error?
    model.requestAssets { result, error in
        assetResult = result
        assetError = error
        semaphore.signal()
    }
    semaphore.wait()
    if let assetError {
        fail("asset request failed: \(assetError)")
    }
    guard assetResult == .available else {
        fail("asset request completed with \(String(describing: assetResult))")
    }
}
guard model.hasAvailableAssets else {
    fail(
        "Latin model assets are not installed; corpus not scored (rerun with --request-assets to ask macOS to download them)"
    )
}
do {
    try model.load()
} catch {
    fail("cannot load Latin model: \(error)")
}
defer {
    model.unload()
}

var top1Relevant = 0
var pairwiseCorrect = 0
var pairwiseTotal = 0
var ndcgTotal = 0.0

for query in corpus.queries {
    let queryVector = meanPooledVector(
        model: model,
        text: query.text,
        language: query.language
    )
    var ranked = query.candidates.map { candidate -> RankedCandidate in
        RankedCandidate(
            candidate: candidate,
            score: cosine(
                queryVector,
                meanPooledVector(
                    model: model,
                    text: candidate.text,
                    language: candidate.language
                )
            )
        )
    }
    ranked.sort { $0.score > $1.score }
    if ranked[0].candidate.relevance >= 2 {
        top1Relevant += 1
    }

    for left in ranked.indices {
        for right in ranked.indices where right > left {
            let first = ranked[left].candidate
            let second = ranked[right].candidate
            if first.relevance == second.relevance {
                continue
            }
            pairwiseTotal += 1
            if first.relevance > second.relevance {
                pairwiseCorrect += 1
            }
        }
    }

    let actualDcg = dcg(ranked.map(\.candidate.relevance))
    let idealDcg = dcg(query.candidates.map(\.relevance).sorted(by: >))
    let ndcg = idealDcg == 0 ? 1 : actualDcg / idealDcg
    ndcgTotal += ndcg
    let top = ranked[0]
    print(
        "\(query.id): top=\(top.candidate.id) score=\(String(format: "%.3f", top.score)) judgement=\(top.candidate.relevance) nDCG=\(String(format: "%.3f", ndcg))"
    )
    for candidate in ranked {
        let padded = candidate.candidate.id.padding(
            toLength: 28,
            withPad: " ",
            startingAt: 0
        )
        print(
            "  \(padded) score=\(String(format: "%.3f", candidate.score)) judgement=\(candidate.candidate.relevance) lang=\(candidate.candidate.language)"
        )
    }
}

let top1Rate = Double(top1Relevant) / Double(corpus.queries.count)
let pairwiseAccuracy = pairwiseTotal == 0
    ? 1 : Double(pairwiseCorrect) / Double(pairwiseTotal)
let meanNdcg = ndcgTotal / Double(corpus.queries.count)
let accepted =
    top1Rate >= corpus.acceptance.minTop1Relevant
    && pairwiseAccuracy >= corpus.acceptance.minPairwiseAccuracy
    && meanNdcg >= corpus.acceptance.minMeanNdcg

print(
    "top1-relevant=\(String(format: "%.3f", top1Rate)) pairwise=\(String(format: "%.3f", pairwiseAccuracy)) mean-nDCG=\(String(format: "%.3f", meanNdcg)) result=\(accepted ? "PASS" : "FAIL")"
)
if !accepted {
    exit(1)
}
