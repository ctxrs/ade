import Foundation

let preferredEffortOrder: [String] = ["none", "minimal", "low", "medium", "high", "xhigh"]

struct ParsedModelId {
    let full: String
    let base: String
    let effort: String?
}

struct ModelCatalog {
    let baseIds: [String]
    let displayNameByBase: [String: String]
    let effortsByBase: [String: [String]]
    let fullIdByBaseEffort: [String: [String: String]]
}

func buildModelCatalog(_ modelIds: [String]) -> ModelCatalog {
    var baseIdsSet = Set<String>()
    var rawEffortsByBase: [String: Set<String>] = [:]
    var rawNamesByBase: [String: [String]] = [:]
    var displayNameByBase: [String: String] = [:]
    var fullIdByBaseEffort: [String: [String: String]] = [:]

    for id in modelIds {
        let trimmedId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedId.isEmpty else { continue }
        let name = trimmedId
        let parts = splitOnLastSlash(trimmedId)
        guard !parts.base.isEmpty else { continue }

        let effort = parts.suffix.flatMap { suffix in
            (isPreferredEffortId(suffix) || hasTrailingParenSuffix(name: name, suffix: suffix)) ? suffix : nil
        }
        let base = effort == nil ? trimmedId : parts.base

        baseIdsSet.insert(base)
        rawNamesByBase[base, default: []].append(name)
        if let effort {
            rawEffortsByBase[base, default: []].insert(effort)
            var map = fullIdByBaseEffort[base] ?? [:]
            map[effort] = trimmedId
            fullIdByBaseEffort[base] = map
        }
    }

    let baseIds = baseIdsSet.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    var effortsByBase: [String: [String]] = [:]
    for base in baseIds {
        let ordered = rawEffortsByBase[base].map(orderEffortIds) ?? []
        let efforts = ordered.count >= 2 ? ordered : []
        effortsByBase[base] = efforts

        if !efforts.isEmpty {
            displayNameByBase[base] = base
            continue
        }

        let names = rawNamesByBase[base] ?? []
        let stripped = names.map { stripTrailingParenIfEffort($0, effortIds: efforts) }
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        displayNameByBase[base] = stripped.first ?? base
    }

    return ModelCatalog(
        baseIds: baseIds,
        displayNameByBase: displayNameByBase,
        effortsByBase: effortsByBase,
        fullIdByBaseEffort: fullIdByBaseEffort
    )
}

func parseModelId(_ fullModelId: String, catalog: ModelCatalog?) -> ParsedModelId {
    let parts = splitOnLastSlash(fullModelId)
    guard !parts.full.isEmpty else { return ParsedModelId(full: "", base: "", effort: nil) }
    guard let suffix = parts.suffix else {
        return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
    }

    if let catalog {
        let options = catalog.effortsByBase[parts.base] ?? []
        if options.contains(suffix) {
            return ParsedModelId(full: parts.full, base: parts.base, effort: suffix)
        }
        if catalog.baseIds.contains(parts.full) {
            return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
        }
    }

    if isPreferredEffortId(suffix) {
        return ParsedModelId(full: parts.full, base: parts.base, effort: suffix)
    }
    return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
}

func composeModelId(base: String, effort: String?) -> String {
    let trimmed = base.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return "" }
    guard let effort, !effort.isEmpty else { return trimmed }
    return "\(trimmed)/\(effort)"
}

func formatEffortLabel(_ effort: String?) -> String {
    let raw = effort?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    guard !raw.isEmpty else { return "" }
    let normalized = raw.lowercased()
    if normalized == "xhigh" || normalized == "extra_high" || normalized == "extra-high" {
        return "Extra High"
    }
    return normalized.prefix(1).uppercased() + normalized.dropFirst()
}

func pickDefaultEffort(_ efforts: [String]) -> String? {
    if let medium = efforts.first(where: { normalizeEffortIdForCompare($0) == "medium" }) {
        return medium
    }
    return efforts.first
}

func deriveFullModelIdForBase(catalog: ModelCatalog, baseId: String, preferredEffort: String?) -> String {
    let efforts = catalog.effortsByBase[baseId] ?? []
    guard !efforts.isEmpty else { return baseId }
    let resolvedEffort = preferredEffort.flatMap { pref in
        efforts.first(where: { normalizeEffortIdForCompare($0) == normalizeEffortIdForCompare(pref) })
    } ?? pickDefaultEffort(efforts)
    guard let effort = resolvedEffort else { return baseId }
    if let fullId = catalog.fullIdByBaseEffort[baseId]?[effort] {
        return fullId
    }
    return composeModelId(base: baseId, effort: effort)
}

func normalizeEffortIdForCompare(_ value: String) -> String {
    value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
}

private func splitOnLastSlash(_ fullModelId: String) -> (full: String, base: String, suffix: String?) {
    let full = fullModelId.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !full.isEmpty else { return ("", "", nil) }
    guard let idx = full.lastIndex(of: "/"), idx != full.startIndex else {
        return (full, full, nil)
    }
    let base = String(full[..<idx])
    let suffix = String(full[full.index(after: idx)...]).trimmingCharacters(in: .whitespacesAndNewlines)
    return (full, base, suffix.isEmpty ? nil : suffix)
}

private func isPreferredEffortId(_ value: String) -> Bool {
    let normalized = normalizeEffortIdForCompare(value)
    return preferredEffortOrder.contains(normalized)
}

private func hasTrailingParenSuffix(name: String, suffix: String) -> Bool {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasSuffix(")") else { return false }
    guard let openIdx = trimmed.lastIndex(of: "("),
          openIdx < trimmed.index(before: trimmed.endIndex) else {
        return false
    }
    let inner = String(trimmed[trimmed.index(after: openIdx)..<trimmed.index(before: trimmed.endIndex)])
    return normalizeEffortIdForCompare(inner) == normalizeEffortIdForCompare(suffix)
}

private func stripTrailingParenIfEffort(_ name: String, effortIds: [String]) -> String {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasSuffix(")") else { return trimmed }
    guard let openIdx = trimmed.lastIndex(of: "("),
          openIdx < trimmed.index(before: trimmed.endIndex) else {
        return trimmed
    }
    let inner = String(trimmed[trimmed.index(after: openIdx)..<trimmed.index(before: trimmed.endIndex)])
    let normalizedInner = normalizeEffortIdForCompare(inner)
    let isEffort = effortIds.contains { normalizeEffortIdForCompare($0) == normalizedInner }
    return isEffort ? String(trimmed[..<openIdx]).trimmingCharacters(in: .whitespacesAndNewlines) : trimmed
}

private func orderEffortIds(_ efforts: Set<String>) -> [String] {
    let list = efforts.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    let orderIndex: (String) -> Int = { value in
        let normalized = normalizeEffortIdForCompare(value)
        return preferredEffortOrder.firstIndex(of: normalized) ?? Int.max
    }
    return list.sorted { left, right in
        let leftIndex = orderIndex(left)
        let rightIndex = orderIndex(right)
        if leftIndex != rightIndex {
            return leftIndex < rightIndex
        }
        return left.localizedCaseInsensitiveCompare(right) == .orderedAscending
    }
}
