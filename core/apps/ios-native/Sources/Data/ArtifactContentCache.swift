import CryptoKit
import Foundation

struct ArtifactPrefetchTarget: Sendable {
    let key: String
    let path: String
    let size: Int
    let fileExtension: String?
}

actor ArtifactContentCache {
    private struct Entry: Codable {
        let key: String
        let filename: String
        let size: Int
        var lastAccess: TimeInterval
    }

    static let shared = ArtifactContentCache()

    private let maxBytes: Int
    private let directory: URL
    private let indexURL: URL
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private var entries: [String: Entry] = [:]
    private var totalBytes = 0
    private var isLoaded = false
    private var inflight: [String: Int] = [:]
    private var activeScope: String?

    init(cacheName: String = "artifact-content", maxBytes: Int = 64 * 1024 * 1024) {
        self.maxBytes = maxBytes
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        directory = base.appendingPathComponent("ctx.ios.cache").appendingPathComponent(cacheName)
        indexURL = directory.appendingPathComponent("index.json")
    }

    func setActiveScope(_ scope: String?) {
        ensureLoaded()
        guard scope != activeScope else { return }
        clear()
        activeScope = scope
    }

    func cachedData(for key: String) -> Data? {
        guard isArtifactPath(key) else { return nil }
        guard activeScope != nil else { return nil }
        ensureLoaded()
        guard var entry = entries[key] else { return nil }
        let url = directory.appendingPathComponent(entry.filename)
        guard let data = try? Data(contentsOf: url) else {
            removeEntry(entry)
            saveIndex()
            return nil
        }
        entry.lastAccess = Date().timeIntervalSince1970
        entries[key] = entry
        saveIndex()
        return data
    }

    func cachedURL(for key: String) -> URL? {
        guard isArtifactPath(key) else { return nil }
        guard activeScope != nil else { return nil }
        ensureLoaded()
        guard var entry = entries[key] else { return nil }
        let url = directory.appendingPathComponent(entry.filename)
        guard FileManager.default.fileExists(atPath: url.path) else {
            removeEntry(entry)
            saveIndex()
            return nil
        }
        entry.lastAccess = Date().timeIntervalSince1970
        entries[key] = entry
        saveIndex()
        return url
    }

    func prefetch(targets: [ArtifactPrefetchTarget], client: DaemonAPIClient) async {
        ensureLoaded()
        guard let scope = activeScope else { return }
        let inflightBytes = inflight.values.reduce(0, +)
        var remaining = maxBytes - totalBytes - inflightBytes
        if remaining <= 0 { return }

        for target in targets {
            if activeScope != scope { return }
            if target.size <= 0 || target.size > maxBytes || target.size > remaining {
                continue
            }
            if entries[target.key] != nil || inflight[target.key] != nil {
                continue
            }
            inflight[target.key] = target.size
            remaining -= target.size
            do {
                let data = try await client.fetchAssetData(path: target.path)
                store(data, for: target.key, fileExtension: target.fileExtension)
            } catch {
                inflight.removeValue(forKey: target.key)
                continue
            }
            inflight.removeValue(forKey: target.key)
        }
    }

    func store(_ data: Data, for key: String, fileExtension: String?) {
        guard isArtifactPath(key) else { return }
        guard activeScope != nil else { return }
        ensureLoaded()
        if data.isEmpty || data.count > maxBytes { return }

        let filename = entries[key]?.filename ?? Self.filename(for: key, fileExtension: fileExtension)
        let url = directory.appendingPathComponent(filename)
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            return
        }

        if let existing = entries[key] {
            totalBytes -= existing.size
        }
        let entry = Entry(
            key: key,
            filename: filename,
            size: data.count,
            lastAccess: Date().timeIntervalSince1970
        )
        entries[key] = entry
        totalBytes += data.count
        evictIfNeeded()
        saveIndex()
    }

    private func clear() {
        let stored = Array(entries.values)
        for entry in stored {
            removeEntry(entry)
        }
        entries.removeAll()
        totalBytes = 0
        inflight.removeAll()
        saveIndex()
    }

    private func evictIfNeeded() {
        while totalBytes > maxBytes, let oldest = entries.values.min(by: { $0.lastAccess < $1.lastAccess }) {
            removeEntry(oldest)
        }
    }

    private func removeEntry(_ entry: Entry) {
        let url = directory.appendingPathComponent(entry.filename)
        try? FileManager.default.removeItem(at: url)
        totalBytes -= entry.size
        entries.removeValue(forKey: entry.key)
    }

    private func ensureLoaded() {
        guard !isLoaded else { return }
        isLoaded = true
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        guard let data = try? Data(contentsOf: indexURL),
              let stored = try? decoder.decode([Entry].self, from: data) else { return }
        entries = Dictionary(uniqueKeysWithValues: stored.map { ($0.key, $0) })
        totalBytes = stored.reduce(0) { $0 + $1.size }
    }

    private func saveIndex() {
        let stored = Array(entries.values)
        guard let data = try? encoder.encode(stored) else { return }
        try? data.write(to: indexURL, options: .atomic)
    }

    private static func filename(for key: String, fileExtension: String?) -> String {
        let hash = SHA256.hash(data: Data(key.utf8))
        let hex = hash.map { String(format: "%02x", $0) }.joined()
        guard let fileExtension, !fileExtension.isEmpty else { return hex }
        let cleaned = fileExtension
            .trimmingCharacters(in: CharacterSet(charactersIn: "."))
            .lowercased()
            .filter { $0.isLetter || $0.isNumber }
        if cleaned.isEmpty { return hex }
        return "\(hex).\(cleaned)"
    }

    private func isArtifactPath(_ path: String) -> Bool {
        return path.hasPrefix("/api/artifacts/")
    }
}
