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
    FileHandle.standardError.write(Data("apple relevance eval: \(message)\n".utf8))
    exit(1)
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

func nlLanguage(for language: String) -> NLLanguage {
    switch language {
    case "de":
        return .german
    case "en":
        return .english
    default:
        fail("unsupported query language \(language)")
    }
}

let defaultCorpus = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("relevance-corpus.json")
let corpusURL = CommandLine.arguments.dropFirst().first
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

print("Comms DE/EN relevance eval · Apple NLEmbedding")
var models = [String: NLEmbedding]()
for language in ["de", "en"] {
    let nativeLanguage = nlLanguage(for: language)
    let revisions = Array(
        NLEmbedding.supportedSentenceEmbeddingRevisions(for: nativeLanguage)
    )
    let currentRevision = NLEmbedding.currentSentenceEmbeddingRevision(for: nativeLanguage)
    let model = NLEmbedding.sentenceEmbedding(for: nativeLanguage)
    if let model {
        models[language] = model
        print(
            "  language=\(language) available=yes current=\(currentRevision) supported=\(revisions) loaded=\(model.revision) dimensions=\(model.dimension)"
        )
    } else {
        print(
            "  language=\(language) available=no current=\(currentRevision) supported=\(revisions)"
        )
    }
}
let missingLanguages = ["de", "en"].filter { models[$0] == nil }
if !missingLanguages.isEmpty {
    fail(
        "sentence embedding unavailable for \(missingLanguages.joined(separator: ", ")); corpus not scored"
    )
}

var top1Relevant = 0
var pairwiseCorrect = 0
var pairwiseTotal = 0
var ndcgTotal = 0.0

for query in corpus.queries {
    guard let model = models[query.language] else {
        fail("unsupported query language \(query.language)")
    }
    guard let queryVector = model.vector(for: query.text) else {
        fail("no \(query.language) query vector for \(query.id)")
    }

    var ranked = query.candidates.map { candidate -> RankedCandidate in
        guard let candidateVector = model.vector(for: candidate.text) else {
            fail(
                "the \(query.language) model produced no vector for \(candidate.id) (\(candidate.language))"
            )
        }
        return RankedCandidate(
            candidate: candidate,
            score: cosine(queryVector, candidateVector)
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
