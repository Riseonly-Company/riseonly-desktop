// Writes crates/rise-i18n/src/number_format/percent_format_data.rs from Foundation itself.
//
// The desktop twin of `NumberFormatter` with `numberStyle = .percent` must print what iOS prints
// for `Locale(identifier: LanguageStore.code)`, and the only exact source of that is Foundation.
// For every catalog language code (and the root locale) this script reads the percent affixes,
// the number symbols and the grouping of a percent formatter, then re-assembles a set of values
// under several fraction and significant digit settings with the very algorithm of
// percent_number_formatter.rs and refuses to write anything unless each result equals
// Foundation's string.
//
//   swift crates/rise-i18n/tools/gen_percent_format.swift            write the data file
//   swift crates/rise-i18n/tools/gen_percent_format.swift --check    verify only, write nothing
//
// Run from the repository root on macOS. Language codes come from the rise-i18n-catalog crate of T.

import Foundation

let root = FileManager.default.currentDirectoryPath
let catalogPath = root + "/../riseonly-tools/crates/rise-i18n-catalog/src/generated.rs"
let outputPath = root + "/crates/rise-i18n/src/number_format/percent_format_data.rs"

let checkValues: [Double] = [
    0, -0.0, 0.0035, -0.0035, 0.000049, 0.00005, 0.000015, 0.000025, 0.0001, 0.001, 0.005, 0.015,
    0.025, 0.07, 0.125, 0.135, 0.2, 0.333333, 2.0 / 3.0, 0.5, 0.995, 0.9995, 0.99999, 1, 1.5,
    12.345, 1234.5, 123456.789, 1e-7, -1e-7, 1e-12, -0.5, -12.345, 0.0000001234,
    .nan, .infinity, -.infinity,
]

struct Settings {
    var minimumFractionDigits: Int
    var maximumFractionDigits: Int
    var usesSignificantDigits: Bool
    var minimumSignificantDigits: Int
    var maximumSignificantDigits: Int
}

let checkSettings: [Settings] = [
    Settings(minimumFractionDigits: 0, maximumFractionDigits: 0, usesSignificantDigits: false, minimumSignificantDigits: 1, maximumSignificantDigits: 6),
    Settings(minimumFractionDigits: 1, maximumFractionDigits: 1, usesSignificantDigits: false, minimumSignificantDigits: 1, maximumSignificantDigits: 6),
    Settings(minimumFractionDigits: 0, maximumFractionDigits: 3, usesSignificantDigits: false, minimumSignificantDigits: 1, maximumSignificantDigits: 6),
    Settings(minimumFractionDigits: 2, maximumFractionDigits: 4, usesSignificantDigits: false, minimumSignificantDigits: 1, maximumSignificantDigits: 6),
    Settings(minimumFractionDigits: 0, maximumFractionDigits: 0, usesSignificantDigits: true, minimumSignificantDigits: 1, maximumSignificantDigits: 2),
    Settings(minimumFractionDigits: 1, maximumFractionDigits: 1, usesSignificantDigits: true, minimumSignificantDigits: 1, maximumSignificantDigits: 2),
    Settings(minimumFractionDigits: 0, maximumFractionDigits: 0, usesSignificantDigits: true, minimumSignificantDigits: 3, maximumSignificantDigits: 4),
]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func catalogCodes() -> [String] {
    guard let text = try? String(contentsOfFile: catalogPath, encoding: .utf8) else {
        fail("cannot read \(catalogPath); run from the repository root")
    }
    let regex = try! NSRegularExpression(pattern: "\\bcode: \"([^\"]+)\"")
    let range = NSRange(text.startIndex..., in: text)
    let found = regex.matches(in: text, range: range).compactMap { match -> String? in
        Range(match.range(at: 1), in: text).map { String(text[$0]) }
    }
    if found.isEmpty { fail("no code in \(catalogPath)") }
    return found
}

struct LocaleData {
    var code: String
    var digits: [Unicode.Scalar]
    var decimalSeparator: [Unicode.Scalar]
    var groupingSeparator: [Unicode.Scalar]
    var usesGrouping: Bool
    var primaryGrouping: Int
    var secondaryGrouping: Int
    var minimumGroupingDigits: Int
    var positivePrefix: [Unicode.Scalar]
    var positiveSuffix: [Unicode.Scalar]
    var negativePrefix: [Unicode.Scalar]
    var negativeSuffix: [Unicode.Scalar]
    var positiveInfinity: [Unicode.Scalar]
    var negativeInfinity: [Unicode.Scalar]
    var notANumber: [Unicode.Scalar]
}

func foundationFormatter(_ code: String, _ settings: Settings) -> NumberFormatter {
    let formatter = NumberFormatter()
    formatter.locale = Locale(identifier: code)
    formatter.numberStyle = .percent
    formatter.minimumFractionDigits = settings.minimumFractionDigits
    formatter.maximumFractionDigits = settings.maximumFractionDigits
    if settings.usesSignificantDigits {
        formatter.usesSignificantDigits = true
        formatter.minimumSignificantDigits = settings.minimumSignificantDigits
        formatter.maximumSignificantDigits = settings.maximumSignificantDigits
    }
    return formatter
}

func build(_ code: String) -> LocaleData {
    let formatter = NumberFormatter()
    formatter.locale = Locale(identifier: code)
    formatter.numberStyle = .percent
    let digits: [Unicode.Scalar] = (0...9).map { digit in
        let text = formatter.string(from: NSNumber(value: Double(digit) / 100)) ?? ""
        guard let scalar = text.unicodeScalars.last(where: { $0.properties.numericType != nil }) else {
            fail("no digit \(digit) for \(code)")
        }
        return scalar
    }
    let primary = formatter.groupingSize
    let secondary = formatter.secondaryGroupingSize > 0 ? formatter.secondaryGroupingSize : primary
    let grouping = Array(formatter.groupingSeparator.unicodeScalars)
    let probe = formatter.string(from: NSNumber(value: 12.34)) ?? ""
    let minimumGrouping = probe.unicodeScalars.contains(where: { grouping.contains($0) }) ? 1 : 2
    return LocaleData(
        code: code,
        digits: digits,
        decimalSeparator: Array(formatter.decimalSeparator.unicodeScalars),
        groupingSeparator: grouping,
        usesGrouping: formatter.usesGroupingSeparator,
        primaryGrouping: primary,
        secondaryGrouping: secondary,
        minimumGroupingDigits: minimumGrouping,
        positivePrefix: Array(formatter.positivePrefix.unicodeScalars),
        positiveSuffix: Array(formatter.positiveSuffix.unicodeScalars),
        negativePrefix: Array(formatter.negativePrefix.unicodeScalars),
        negativeSuffix: Array(formatter.negativeSuffix.unicodeScalars),
        positiveInfinity: Array(formatter.positiveInfinitySymbol.unicodeScalars),
        negativeInfinity: Array(formatter.negativeInfinitySymbol.unicodeScalars),
        notANumber: Array(formatter.notANumberSymbol.unicodeScalars)
    )
}

// Mirror of `PlainDecimal` in percent_number_formatter.rs: value = 0.d1d2d3... * 10^point,
// no leading or trailing zero digit, no digit at all for zero.
struct PlainDecimal {
    var digits: [Int]
    var point: Int
}

// Mirror of `PlainDecimal::shortest`. Keep the two identical.
func shortest(_ magnitude: Double) -> PlainDecimal {
    let text = "\(magnitude)"
    let parts = text.split(separator: "e", omittingEmptySubsequences: false)
    let mantissa = String(parts[0])
    let exponent = parts.count > 1 ? (Int(parts[1]) ?? 0) : 0
    let pieces = mantissa.split(separator: ".", omittingEmptySubsequences: false)
    let integer = Array(pieces[0])
    let fraction = pieces.count > 1 ? Array(pieces[1]) : []
    var digits = (integer + fraction).compactMap { $0.wholeNumberValue }
    var point = integer.count + exponent
    while digits.first == 0 {
        digits.removeFirst()
        point -= 1
    }
    while digits.last == 0 { digits.removeLast() }
    if digits.isEmpty { point = 0 }
    return PlainDecimal(digits: digits, point: point)
}

// Mirror of `PlainDecimal::round_half_even`. Keep the two identical.
func roundHalfEven(_ plain: inout PlainDecimal, _ keep: Int) {
    if keep >= plain.digits.count { return }
    if keep < 0 {
        plain = PlainDecimal(digits: [], point: 0)
        return
    }
    let firstDropped = plain.digits[keep]
    let restNonZero = plain.digits[(keep + 1)...].contains(where: { $0 != 0 })
    let lastKeptOdd = keep > 0 && plain.digits[keep - 1] % 2 == 1
    let up = firstDropped > 5 || (firstDropped == 5 && (restNonZero || lastKeptOdd))
    plain.digits.removeLast(plain.digits.count - keep)
    if up {
        var carry = true
        var index = plain.digits.count - 1
        while index >= 0 && carry {
            if plain.digits[index] == 9 {
                plain.digits[index] = 0
            } else {
                plain.digits[index] += 1
                carry = false
            }
            index -= 1
        }
        if carry {
            plain.digits.insert(1, at: 0)
            plain.point += 1
        }
    }
    while plain.digits.last == 0 { plain.digits.removeLast() }
    if plain.digits.isEmpty { plain.point = 0 }
}

// Mirror of `PercentNumberFormatter::string` in percent_number_formatter.rs. Keep the two identical.
func assemble(_ value: Double, _ settings: Settings, _ data: LocaleData) -> String {
    var out: [Unicode.Scalar] = []
    if value.isNaN {
        out = data.notANumber
    } else if value.isInfinite {
        out = value < 0 ? data.negativeInfinity : data.positiveInfinity
    } else {
        var plain = shortest(abs(value))
        if !plain.digits.isEmpty { plain.point += 2 }
        var minimumFraction = 0
        var significantPad = 0
        if settings.usesSignificantDigits {
            let minimum = max(settings.minimumSignificantDigits, 1)
            let maximum = max(settings.maximumSignificantDigits, minimum)
            roundHalfEven(&plain, maximum)
            let shown = plain.digits.isEmpty ? 1 : max(plain.digits.count, plain.point)
            significantPad = max(minimum - shown, 0)
        } else {
            let maximum = max(settings.maximumFractionDigits, settings.minimumFractionDigits)
            roundHalfEven(&plain, plain.point + maximum)
            minimumFraction = settings.minimumFractionDigits
        }

        var integer: [Int] = []
        if plain.point > 0 {
            integer = Array(plain.digits.prefix(plain.point))
            while integer.count < plain.point { integer.append(0) }
        } else {
            integer = [0]
        }
        var fraction: [Int] = []
        if plain.point < 0 {
            fraction = Array(repeating: 0, count: -plain.point) + plain.digits
        } else if plain.point < plain.digits.count {
            fraction = Array(plain.digits[plain.point...])
        }
        minimumFraction = max(minimumFraction, fraction.count + significantPad)
        while fraction.count < minimumFraction { fraction.append(0) }

        let negative = value.sign == .minus
        out += negative ? data.negativePrefix : data.positivePrefix
        let len = integer.count
        if data.usesGrouping && data.primaryGrouping > 0 && len >= data.primaryGrouping + data.minimumGroupingDigits {
            var cuts: [Int] = []
            var end = len - data.primaryGrouping
            cuts.append(end)
            let secondary = max(data.secondaryGrouping, 1)
            while end > secondary {
                end -= secondary
                cuts.append(end)
            }
            var start = 0
            for cut in cuts.reversed() {
                out += integer[start..<cut].map { data.digits[$0] }
                out += data.groupingSeparator
                start = cut
            }
            out += integer[start...].map { data.digits[$0] }
        } else {
            out += integer.map { data.digits[$0] }
        }
        if !fraction.isEmpty {
            out += data.decimalSeparator
            out += fraction.map { data.digits[$0] }
        }
        out += negative ? data.negativeSuffix : data.positiveSuffix
    }
    var text = ""
    text.unicodeScalars.append(contentsOf: out)
    return text
}

func verify(_ data: LocaleData) -> Int {
    var checked = 0
    for settings in checkSettings {
        let formatter = foundationFormatter(data.code, settings)
        for value in checkValues {
            let expected = formatter.string(from: NSNumber(value: value)) ?? ""
            let actual = assemble(value, settings, data)
            if expected != actual {
                fail("\(data.code.isEmpty ? "root" : data.code) \(value) \(settings): Foundation \"\(expected)\", assembled \"\(actual)\"")
            }
            checked += 1
        }
    }
    return checked
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

func rustChar(_ scalar: Unicode.Scalar) -> String {
    scalar.value < 0x7F ? "'\(scalar)'" : String(format: "'\\u{%x}'", scalar.value)
}

func render(_ all: [LocaleData]) -> String {
    let version = ProcessInfo.processInfo.operatingSystemVersionString
    var out = """
    // @generated by crates/rise-i18n/tools/gen_percent_format.swift from Foundation (macOS \(version)). Do not edit.
    #![cfg_attr(rustfmt, rustfmt_skip)]

    use super::percent_number_formatter::LocalePercentData;

    pub(super) static LOCALES: &[LocalePercentData] = &[

    """
    for data in all {
        out += "    LocalePercentData {\n"
        out += "        code: \"\(data.code)\",\n"
        out += "        digits: [\(data.digits.map(rustChar).joined(separator: ", "))],\n"
        out += "        decimal_separator: \(rustString(data.decimalSeparator)),\n"
        out += "        grouping_separator: \(rustString(data.groupingSeparator)),\n"
        out += "        uses_grouping: \(data.usesGrouping),\n"
        out += "        primary_grouping: \(data.primaryGrouping),\n"
        out += "        secondary_grouping: \(data.secondaryGrouping),\n"
        out += "        minimum_grouping_digits: \(data.minimumGroupingDigits),\n"
        out += "        positive_prefix: \(rustString(data.positivePrefix)),\n"
        out += "        positive_suffix: \(rustString(data.positiveSuffix)),\n"
        out += "        negative_prefix: \(rustString(data.negativePrefix)),\n"
        out += "        negative_suffix: \(rustString(data.negativeSuffix)),\n"
        out += "        positive_infinity: \(rustString(data.positiveInfinity)),\n"
        out += "        negative_infinity: \(rustString(data.negativeInfinity)),\n"
        out += "        not_a_number: \(rustString(data.notANumber)),\n"
        out += "    },\n"
    }
    out += "];\n"
    return out
}

let arguments = Array(CommandLine.arguments.dropFirst())
// The root locale ("") is what Foundation falls back to for an identifier it does not know.
let codes = [""] + catalogCodes()
var all: [LocaleData] = []
var checked = 0
for code in codes {
    let data = build(code)
    checked += verify(data)
    all.append(data)
}
let text = render(all)
print("locales \(all.count), checked \(checked) strings, \(text.utf8.count) bytes")
if arguments.first == "--check" { exit(0) }
do {
    try text.write(toFile: outputPath, atomically: true, encoding: .utf8)
} catch {
    fail("cannot write \(outputPath): \(error)")
}
print("wrote \(outputPath)")
