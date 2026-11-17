// Writes crates/rise-i18n/src/number_format/measurement_format_data.rs from Foundation itself.
//
// The desktop twin of `MeasurementFormatter` must print what iOS prints, and the only exact source
// of that is Apple's Foundation. For every canon locale, every unit the app formats and every
// unit style this script asks Foundation to format a fixed list of sample counts, cuts the number
// out of each answer (the rest is the unit pattern of that plural form, "{0} km"), and
// takes the decimal number symbols from NumberFormatter. It then re-assembles every sample and a
// set of other values with the very algorithm of measurement_formatter.rs and refuses to write
// anything unless each result equals Foundation's string.
//
// Patterns are kept per CLDR plural category of the shown number. The rules live in this script
// and in measurement_formatter.rs; a rule that disagrees with Foundation's ICU picks a wrong
// pattern for some checked value and stops the script.
//
//   swift crates/rise-i18n/tools/gen_measurement_format.swift            write the data file
//   swift crates/rise-i18n/tools/gen_measurement_format.swift --check    verify only, write nothing
//   swift crates/rise-i18n/tools/gen_measurement_format.swift --vectors ru_RU
//                                                                          print Foundation strings
//
// Run from the repository root on macOS. Locales come from the rise-i18n-catalog crate of T.

import Foundation

let root = FileManager.default.currentDirectoryPath
let catalogPath = root + "/../riseonly-tools/crates/rise-i18n-catalog/src/generated.rs"
let outputPath = root + "/crates/rise-i18n/src/number_format/measurement_format_data.rs"
let placeholder = "{0}"

// Keep the order identical to `UnitLength::format_index` and `UnitSpeed::format_index`.
let units: [(name: String, unit: Dimension)] = [
    ("meters", UnitLength.meters),
    ("kilometers", UnitLength.kilometers),
    ("feet", UnitLength.feet),
    ("miles", UnitLength.miles),
    ("kilometersPerHour", UnitSpeed.kilometersPerHour),
    ("milesPerHour", UnitSpeed.milesPerHour),
]

// Keep the order identical to `FormatterUnitStyle::format_index`.
let styles: [(name: String, style: Formatter.UnitStyle)] = [
    ("short", .short),
    ("medium", .medium),
    ("long", .long),
]

// Values a plural pattern is cut from, in this order: the first value of each plural category wins.
let samples: [String] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "10", "11", "12", "13", "14", "15", "19", "20",
    "21", "22", "23", "24", "25", "100", "101", "102", "103", "111", "1000", "1000000",
    "0.5", "1.5", "2.5", "0.1", "0.2", "0.3", "0.4", "0.6", "1.2",
]

// Values checked on top of the samples, each at every fraction digit count below: rounding half
// to even, two shown fraction digits (v = 2), grouping, the minus sign.
let extraChecks: [String] = [
    "0.04", "0.12", "0.21", "0.25", "0.35", "1.25", "2.45", "62.137", "1234.5", "123456",
    "1234567.25", "2000000", "-1", "-5", "-1.5",
]

let fractionDigitChoices = [0, 1, 3]

// Keep the order identical to `pattern_index` in measurement_formatter.rs.
let categories = ["zero", "one", "two", "few", "many", "other"]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func catalogLocales() -> [String] {
    guard let text = try? String(contentsOfFile: catalogPath, encoding: .utf8) else {
        fail("cannot read \(catalogPath); run from the repository root")
    }
    let regex = try! NSRegularExpression(pattern: "locale: \"([^\"]+)\"")
    let range = NSRange(text.startIndex..., in: text)
    let found = regex.matches(in: text, range: range).compactMap { match -> String? in
        Range(match.range(at: 1), in: text).map { String(text[$0]) }
    }
    if found.isEmpty { fail("no locale in \(catalogPath)") }
    return found
}

func decimalValue(_ text: String) -> Double {
    guard let value = Double(text) else { fail("bad sample \(text)") }
    return value
}

var formatterCache: [String: MeasurementFormatter] = [:]

func foundation(_ value: Double, _ unit: Dimension, _ style: Formatter.UnitStyle, _ locale: Locale, _ fractionDigits: Int) -> String {
    let key = "\(locale.identifier)|\(style.rawValue)|\(fractionDigits)"
    let formatter: MeasurementFormatter
    if let cached = formatterCache[key] {
        formatter = cached
    } else {
        formatter = MeasurementFormatter()
        formatter.locale = locale
        formatter.unitOptions = .providedUnit
        formatter.unitStyle = style
        formatter.numberFormatter.maximumFractionDigits = fractionDigits
        formatterCache[key] = formatter
    }
    return formatter.string(from: Measurement(value: value, unit: unit))
}

struct NumberData {
    var digits: [Unicode.Scalar]
    var decimalSeparator: [Unicode.Scalar]
    var groupingSeparator: [Unicode.Scalar]
    var primaryGrouping: Int
    var secondaryGrouping: Int
    var minimumGroupingDigits: Int
    var minusSign: [Unicode.Scalar]
}

func decimalFormatter(_ locale: Locale, _ fractionDigits: Int) -> NumberFormatter {
    let formatter = NumberFormatter()
    formatter.locale = locale
    formatter.numberStyle = .decimal
    formatter.maximumFractionDigits = fractionDigits
    return formatter
}

func numberData(_ locale: Locale) -> NumberData {
    let formatter = decimalFormatter(locale, 3)
    let digits: [Unicode.Scalar] = (0...9).map { digit in
        let text = formatter.string(from: NSNumber(value: digit)) ?? ""
        guard let scalar = text.unicodeScalars.last(where: { $0.properties.numericType != nil }) else {
            fail("no digit \(digit) for \(locale.identifier)")
        }
        return scalar
    }
    let primary = formatter.groupingSize
    let secondary = formatter.secondaryGroupingSize > 0 ? formatter.secondaryGroupingSize : primary
    let grouping = Array(formatter.groupingSeparator.unicodeScalars)
    let probe = formatter.string(from: NSNumber(value: 1234)) ?? ""
    let minimumGrouping = probe.unicodeScalars.contains(where: { grouping.contains($0) }) ? 1 : 2
    return NumberData(digits: digits, decimalSeparator: Array(formatter.decimalSeparator.unicodeScalars),
                      groupingSeparator: grouping, primaryGrouping: primary, secondaryGrouping: secondary,
                      minimumGroupingDigits: minimumGrouping, minusSign: Array(formatter.minusSign.unicodeScalars))
}

// The number as shown, before digits are localized: sign, integer digits, fraction digits.
struct PlainNumber {
    var negative: Bool
    var integer: [Character]
    var fraction: [Character]
}

// Mirror of `rounded_plain` in measurement_formatter.rs. Keep the two identical: the shortest
// decimal text of the double, rounded half to even, trailing zeros dropped.
func roundedPlain(_ value: Double, _ fractionDigits: Int) -> PlainNumber {
    guard var source = Decimal(string: "\(value)", locale: Locale(identifier: "en_US_POSIX")) else {
        fail("no decimal for \(value)")
    }
    var rounded = Decimal()
    NSDecimalRound(&rounded, &source, fractionDigits, .bankers)
    let negative = rounded < 0
    let magnitude = negative ? -rounded : rounded
    let plain = NSDecimalNumber(decimal: magnitude).stringValue
    let pieces = plain.split(separator: ".", omittingEmptySubsequences: false)
    var fraction = pieces.count > 1 ? Array(pieces[1]) : []
    while fraction.last == "0" { fraction.removeLast() }
    return PlainNumber(negative: negative, integer: Array(pieces[0]), fraction: fraction)
}

// Mirror of `format_number` in measurement_formatter.rs. Keep the two identical.
func assembleNumber(_ plain: PlainNumber, _ data: NumberData) -> String {
    var groups: [[Character]] = []
    var rest = plain.integer
    if data.primaryGrouping > 0 && rest.count >= data.primaryGrouping + data.minimumGroupingDigits {
        groups.insert(Array(rest.suffix(data.primaryGrouping)), at: 0)
        rest.removeLast(data.primaryGrouping)
        while rest.count > data.secondaryGrouping {
            groups.insert(Array(rest.suffix(data.secondaryGrouping)), at: 0)
            rest.removeLast(data.secondaryGrouping)
        }
    }
    groups.insert(rest, at: 0)

    func localized(_ characters: [Character]) -> [Unicode.Scalar] {
        characters.map { data.digits[Int(String($0))!] }
    }
    var out: [Unicode.Scalar] = []
    if plain.negative { out += data.minusSign }
    for (index, group) in groups.enumerated() {
        if index > 0 { out += data.groupingSeparator }
        out += localized(group)
    }
    if !plain.fraction.isEmpty {
        out += data.decimalSeparator
        out += localized(plain.fraction)
    }
    var text = ""
    text.unicodeScalars.append(contentsOf: out)
    return text
}

// Mirror of `plural_category` in measurement_formatter.rs, CLDR cardinal rules over the shown
// number: n absolute value, i integer digits, v count and f value of the fraction digits.
func pluralCategory(_ identifier: String, _ plain: PlainNumber) -> Int {
    let iText = String(plain.integer)
    let fText = String(plain.fraction)
    let v = plain.fraction.count
    // Only remainders of i matter for large values; the low 18 digits keep them exact.
    let i = Int(iText.suffix(18)) ?? 0
    let f = v == 0 ? 0 : (Int(fText) ?? 0)
    let nIsInteger = v == 0
    func n(_ k: Int) -> Bool { nIsInteger && i == k }
    let i10 = i % 10, i100 = i % 100, f10 = f % 10, f100 = f % 100
    let million = nIsInteger && i != 0 && i % 1_000_000 == 0
    let zero = 0, one = 1, two = 2, few = 3, many = 4, other = 5
    let language = identifier.split(separator: "_").first.map(String.init) ?? ""
    switch language {
    case "ar":
        if n(0) { return zero }
        if n(1) { return one }
        if n(2) { return two }
        if nIsInteger && (3...10).contains(i100) { return few }
        if nIsInteger && (11...99).contains(i100) { return many }
        return other
    case "bn", "fa", "hi":
        return (i == 0 || n(1)) ? one : other
    case "cs":
        if i == 1 && v == 0 { return one }
        if (2...4).contains(i) && v == 0 { return few }
        if v != 0 { return many }
        return other
    case "da":
        return (n(1) || (f != 0 && (i == 0 || i == 1))) ? one : other
    case "es", "it":
        if language == "es" ? n(1) : (i == 1 && v == 0) { return one }
        return million ? many : other
    case "fr":
        if i == 0 || i == 1 { return one }
        return million ? many : other
    case "pt":
        let isOne = identifier == "pt_PT" ? (i == 1 && v == 0) : (i == 0 || i == 1)
        if isOne { return one }
        return million ? many : other
    case "fil":
        let isOne = (v == 0 && (1...3).contains(i)) || (v == 0 && ![4, 6, 9].contains(i10)) || (v != 0 && ![4, 6, 9].contains(f10))
        return isOne ? one : other
    case "he":
        if (i == 1 && v == 0) || (i == 0 && v != 0) { return one }
        if i == 2 && v == 0 { return two }
        return other
    case "pl":
        if i == 1 && v == 0 { return one }
        if v == 0 && (2...4).contains(i10) && !(12...14).contains(i100) { return few }
        if v == 0 && ((i != 1 && (0...1).contains(i10)) || (5...9).contains(i10) || (12...14).contains(i100)) { return many }
        return other
    case "ro":
        if i == 1 && v == 0 { return one }
        if v != 0 || n(0) || (nIsInteger && i != 1 && (1...19).contains(i100)) { return few }
        return other
    case "ru", "uk":
        if v == 0 && i10 == 1 && i100 != 11 { return one }
        if v == 0 && (2...4).contains(i10) && !(12...14).contains(i100) { return few }
        if v == 0 && (i10 == 0 || (5...9).contains(i10) || (11...14).contains(i100)) { return many }
        return other
    case "sr":
        if (v == 0 && i10 == 1 && i100 != 11) || (f10 == 1 && f100 != 11) { return one }
        if (v == 0 && (2...4).contains(i10) && !(12...14).contains(i100)) || ((2...4).contains(f10) && !(12...14).contains(f100)) { return few }
        return other
    case "de", "en", "fi", "nl", "sv", "sw", "ur":
        return (i == 1 && v == 0) ? one : other
    case "el", "hu", "ka", "kk", "nb", "ta", "tr", "uz":
        return n(1) ? one : other
    default:
        return other
    }
}

// Mirror of the pattern step of `MeasurementFormatter::string`.
func applyPattern(_ pattern: String, _ number: String) -> String {
    guard let range = pattern.range(of: placeholder) else { return pattern }
    return pattern.replacingCharacters(in: range, with: number)
}

struct LocaleData {
    var identifier: String
    var number: NumberData
    // units.count * styles.count rows, each one pattern per plural category.
    var patterns: [[String]]
}

func pattern(_ text: String, number: String, digits: [Unicode.Scalar], context: String) -> String {
    if let range = text.range(of: number) {
        return text.replacingCharacters(in: range, with: placeholder)
    }
    if text.unicodeScalars.contains(where: { digits.contains($0) }) {
        fail("\(context): number \(number.debugDescription) not found in \(text.debugDescription)")
    }
    // Foundation spells some forms without the number ("متران" is "2 meters" in Arabic).
    return text
}

func build(_ identifier: String) -> LocaleData {
    let locale = Locale(identifier: identifier)
    let number = numberData(locale)
    var rows: [[String]] = []
    for unit in units {
        for style in styles {
            var row: [String?] = Array(repeating: nil, count: categories.count)
            for sample in samples {
                let value = decimalValue(sample)
                let plain = roundedPlain(value, 1)
                let category = pluralCategory(identifier, plain)
                if row[category] != nil { continue }
                let text = foundation(value, unit.unit, style.style, locale, 1)
                row[category] = pattern(text, number: assembleNumber(plain, number), digits: number.digits,
                                        context: "\(identifier) \(unit.name) \(style.name) \(sample)")
            }
            guard let otherPattern = row[categories.count - 1] else { fail("\(identifier): no other form") }
            rows.append(row.map { $0 ?? otherPattern })
        }
    }
    return LocaleData(identifier: identifier, number: number, patterns: rows)
}

func verify(_ data: LocaleData) -> Int {
    let locale = Locale(identifier: data.identifier)
    var checked = 0
    for digitsAfter in fractionDigitChoices {
        let formatter = decimalFormatter(locale, digitsAfter)
        for text in samples + extraChecks {
            let value = decimalValue(text)
            let want = formatter.string(from: NSNumber(value: value)) ?? ""
            let got = assembleNumber(roundedPlain(value, digitsAfter), data.number)
            if want != got {
                fail("\(data.identifier) number \(text) at \(digitsAfter): Foundation \(want.debugDescription), twin \(got.debugDescription)")
            }
            checked += 1
        }
    }
    for (unitIndex, unit) in units.enumerated() {
        for (styleIndex, style) in styles.enumerated() {
            let row = data.patterns[unitIndex * styles.count + styleIndex]
            for digitsAfter in fractionDigitChoices {
                for text in samples + extraChecks {
                    let value = decimalValue(text)
                    let plain = roundedPlain(value, digitsAfter)
                    let want = foundation(value, unit.unit, style.style, locale, digitsAfter)
                    let got = applyPattern(row[pluralCategory(data.identifier, plain)], assembleNumber(plain, data.number))
                    if want != got {
                        fail("\(data.identifier) \(unit.name) \(style.name) \(text) at \(digitsAfter): Foundation \(want.debugDescription), twin \(got.debugDescription)")
                    }
                    checked += 1
                }
            }
        }
    }
    return checked
}

func regions(_ system: Locale.MeasurementSystem) -> [String] {
    Locale.Region.isoRegions
        .map(\.identifier)
        .filter { Locale(identifier: "en_" + $0).measurementSystem == system }
        .sorted()
}

func rustString(_ scalars: [Unicode.Scalar]) -> String {
    var out = "\""
    for scalar in scalars {
        switch scalar {
        case "\"": out += "\\\""
        case "\\": out += "\\\\"
        default:
            if scalar.value >= 0x20 && scalar.value < 0x7F {
                out.unicodeScalars.append(scalar)
            } else {
                out += String(format: "\\u{%x}", scalar.value)
            }
        }
    }
    return out + "\""
}

func rustString(_ text: String) -> String {
    rustString(Array(text.unicodeScalars))
}

func rustChar(_ scalar: Unicode.Scalar) -> String {
    scalar.value < 0x7F ? "'\(scalar)'" : String(format: "'\\u{%x}'", scalar.value)
}

final class Interner {
    var strings: [String] = []
    var index: [String: Int] = [:]
    func id(_ text: String) -> Int {
        if let found = index[text] { return found }
        strings.append(text)
        index[text] = strings.count - 1
        return strings.count - 1
    }
}

func render(_ all: [LocaleData]) -> String {
    let version = ProcessInfo.processInfo.operatingSystemVersionString
    let interner = Interner()
    var locales = ""
    for data in all {
        let number = data.number
        locales += "    LocaleMeasurementData {\n"
        locales += "        identifier: \"\(data.identifier)\",\n"
        locales += "        digits: [\(number.digits.map(rustChar).joined(separator: ", "))],\n"
        locales += "        decimal_separator: \(rustString(number.decimalSeparator)),\n"
        locales += "        grouping_separator: \(rustString(number.groupingSeparator)),\n"
        locales += "        primary_grouping: \(number.primaryGrouping),\n"
        locales += "        secondary_grouping: \(number.secondaryGrouping),\n"
        locales += "        minimum_grouping_digits: \(number.minimumGroupingDigits),\n"
        locales += "        minus_sign: \(rustString(number.minusSign)),\n"
        locales += "        patterns: [\n"
        for row in data.patterns {
            locales += "            [\(row.map { String(interner.id($0)) }.joined(separator: ", "))],\n"
        }
        locales += "        ],\n"
        locales += "    },\n"
    }
    var out = """
    // @generated by crates/rise-i18n/tools/gen_measurement_format.swift from Foundation (macOS \(version)). Do not edit.
    #![cfg_attr(rustfmt, rustfmt_skip)]

    use super::measurement_formatter::LocaleMeasurementData;

    pub(super) const PLACEHOLDER: &str = "\(placeholder)";

    pub(super) const CATEGORY_COUNT: usize = \(categories.count);

    pub(super) const PATTERN_ROWS: usize = \(units.count * styles.count);

    pub(super) static US_SYSTEM_REGIONS: &[&str] = &[\(regions(.us).map { "\"\($0)\"" }.joined(separator: ", "))];

    pub(super) static UK_SYSTEM_REGIONS: &[&str] = &[\(regions(.uk).map { "\"\($0)\"" }.joined(separator: ", "))];


    """
    var patterns = "pub(super) static PATTERNS: &[&str] = &["
    for (offset, text) in interner.strings.enumerated() {
        patterns += offset % 6 == 0 ? "\n    " : " "
        patterns += rustString(text) + ","
    }
    out += patterns + "\n];\n\npub(super) static LOCALES: &[LocaleMeasurementData] = &[\n"
    out += locales
    out += "];\n"
    return out
}

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.first == "--vectors" {
    guard arguments.count == 2 else { fail("--vectors <locale>") }
    let locale = Locale(identifier: arguments[1])
    for unit in units {
        for style in styles {
            for text in samples + extraChecks {
                let digitsAfter = text.contains(".") ? 1 : 0
                let shown = foundation(decimalValue(text), unit.unit, style.style, locale, digitsAfter)
                print(arguments[1], unit.name, style.name, text, digitsAfter, rustString(shown))
            }
        }
    }
    exit(0)
}

// The root locale ("") is what Foundation falls back to for an identifier it does not know.
let identifiers = [""] + catalogLocales()
var all: [LocaleData] = []
var checked = 0
for identifier in identifiers {
    let data = build(identifier)
    checked += verify(data)
    all.append(data)
}
let text = render(all)
print("locales \(all.count), units \(units.count), styles \(styles.count), checked \(checked) strings, \(text.utf8.count) bytes")
if arguments.first == "--check" { exit(0) }
do {
    try text.write(toFile: outputPath, atomically: true, encoding: .utf8)
} catch {
    fail("cannot write \(outputPath): \(error)")
}
print("wrote \(outputPath)")
