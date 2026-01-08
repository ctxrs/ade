import Foundation

struct CtxID: Codable, Hashable, Sendable, CustomStringConvertible {
    let rawValue: String

    init(_ rawValue: String) {
        self.rawValue = rawValue
    }

    var description: String {
        rawValue
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let value = try? container.decode(String.self) {
            self.rawValue = value
            return
        }
        if let object = try? container.decode([String: String].self), let value = object["0"] {
            self.rawValue = value
            return
        }
        throw DecodingError.dataCorruptedError(in: container, debugDescription: "Invalid ctx id.")
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

extension CtxID {
    var stringValue: String {
        rawValue
    }
}
