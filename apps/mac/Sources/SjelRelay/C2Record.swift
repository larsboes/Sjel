import Foundation

/// Value type representing heterogeneous attributes in a C2 data record.
public enum C2Value: Codable, Sendable, Equatable {
    case string(String)
    case number(Double)
    case integer(Int)
    case boolean(Bool)
    case data(Data)
    case array([C2Value])
    case object([String: C2Value])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let stringValue = try? container.decode(String.self) {
            self = .string(stringValue)
        } else if let intValue = try? container.decode(Int.self) {
            self = .integer(intValue)
        } else if let doubleValue = try? container.decode(Double.self) {
            self = .number(doubleValue)
        } else if let boolValue = try? container.decode(Bool.self) {
            self = .boolean(boolValue)
        } else if let dataValue = try? container.decode(Data.self) {
            self = .data(dataValue)
        } else if let arrayValue = try? container.decode([C2Value].self) {
            self = .array(arrayValue)
        } else if let objectValue = try? container.decode([String: C2Value].self) {
            self = .object(objectValue)
        } else {
            throw DecodingError.dataCorrupted(
                DecodingError.Context(codingPath: decoder.codingPath, debugDescription: "Unsupported C2Value payload")
            )
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .string(let s):
            try container.encode(s)
        case .integer(let i):
            try container.encode(i)
        case .number(let d):
            try container.encode(d)
        case .boolean(let b):
            try container.encode(b)
        case .data(let d):
            try container.encode(d)
        case .array(let a):
            try container.encode(a)
        case .object(let o):
            try container.encode(o)
        }
    }
}

/// A C2 (sensitive private entity) record in Sjel.
/// Under Product Rule 4, this data must never be transmitted off the owner's devices
/// without end-to-end encryption or pseudonymization.
public struct C2Record: Codable, Sendable, Equatable {
    public let id: String
    public let entityType: String
    public let createdAt: Date
    public let updatedAt: Date
    public var attributes: [String: C2Value]

    public init(
        id: String,
        entityType: String,
        createdAt: Date = Date(),
        updatedAt: Date = Date(),
        attributes: [String: C2Value] = [:]
    ) {
        self.id = id
        self.entityType = entityType
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.attributes = attributes
    }
}
