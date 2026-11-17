// Writes crates/rise-i18n/src/number_format/currency_format_data.rs from Foundation itself.
//
// The desktop twin of `Decimal.FormatStyle.Currency` must print exactly what iOS prints, and the
// only exact source of that is Apple's Foundation (its ICU build and CLDR revision), not CLDR
// read through another library. So this script asks Foundation on macOS for every canon locale
// and every common ISO currency, splits each answer into the parts the Rust formatter assembles
// (digits, separators, grouping, minus, prefix and suffix), then re-assembles a set of amounts
// with the very algorithm of decimal_format_style_currency.rs and refuses to write anything unless every
// result equals Foundation's string.
//
//   swift crates/rise-i18n/tools/gen_currency_format.swift            write the data file
//   swift crates/rise-i18n/tools/gen_currency_format.swift --check    verify only, write nothing
//   swift crates/rise-i18n/tools/gen_currency_format.swift --vectors ru_RU USD,KZT,EUR
//                                                                       print Foundation strings
//
// Run from the repository root on macOS. Locales come from app_language_catalog.rs.

import Foundation

typealias Presentation = CurrencyFormatStyleConfiguration.Presentation

let root = FileManager.default.currentDirectoryPath
let catalogPath = root + "/crates/rise-i18n/src/app_language_catalog.rs"
let outputPath = root + "/crates/rise-i18n/src/number_format/currency_format_data.rs"

// Codes Foundation does not know are printed with the code itself in place of the symbol, so one
// template per locale built on a placeholder code covers them; two other codes verify it.
let templateCode = "QQQ"
let templateChecks = ["ZZZ", "ABC"]

let samples: [String] = [
    "0", "0.5", "1", "2.5", "3.5", "2.99", "0.125", "0.135", "9.995", "1490", "12345", "123456",
    "1234567.895", "99999999.99", "-2.99", "-1490", "-0.5", "-1234567.89",
]

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

func catalogLocales() -> [String] {
    guard let text = try? String(contentsOfFile: catalogPath, encoding: .utf8) else {
        fail("cannot read \(catalogPath); run from the repository root")
    }
    let regex = try! NSRegularExpression(pattern: "locale_identifier: \"([^\"]+)\"")
    let range = NSRange(text.startIndex..., in: text)
    let found = regex.matches(in: text, range: range).compactMap { match -> String? in
        Range(match.range(at: 1), in: text).map { String(text[$0]) }
    }
    if found.isEmpty { fail("no locale_identifier in \(catalogPath)") }
    return found
}

func format(_ value: Decimal, _ code: String, _ presentation: Presentation, _ locale: Locale) -> String {
    value.formatted(.currency(code: code).presentation(presentation).locale(locale))
}

struct Affixes: Equatable {
    var fractionDigits: Int
    var prefix: [Unicode.Scalar]
    var suffix: [Unicode.Scalar]
}

struct LocaleData {
    var identifier: String
    var digits: [Unicode.Scalar]
    var decimalSeparator: [Unicode.Scalar]
    var groupingSeparator: [Unicode.Scalar]
    var primaryGrouping: Int
    var secondaryGrouping: Int
    var minimumGroupingDigits: Int
    var minusSign: [Unicode.Scalar]
    var minusBeforePrefix: Bool
    var template: (narrow: Affixes, standard: Affixes)
    var currencies: [(code: String, narrow: Affixes, standard: Affixes)]
}

func localeDigits(_ locale: Locale) -> [Unicode.Scalar] {
    let formatter = NumberFormatter()
    formatter.locale = locale
    formatter.numberStyle = .decimal
    return (0...9).map { digit in
        let text = formatter.string(from: NSNumber(value: digit)) ?? ""
        guard let scalar = text.unicodeScalars.last(where: { $0.properties.numericType != nil }) else {
            fail("no digit \(digit) for \(locale.identifier)")
        }
        return scalar
    }
}

// Splits a formatted string around its numeric core: everything before the first digit and
// everything after the last one.
func split(_ text: String, digits: [Unicode.Scalar]) -> (left: [Unicode.Scalar], core: [Unicode.Scalar], right: [Unicode.Scalar]) {
    let scalars = Array(text.unicodeScalars)
    guard let first = scalars.firstIndex(where: { digits.contains($0) }),
          let last = scalars.lastIndex(where: { digits.contains($0) }) else {
        fail("no digits in \(text.debugDescription)")
    }
    return (Array(scalars[..<first]), Array(scalars[first...last]), Array(scalars[(last + 1)...]))
}

func affixes(_ code: String, _ presentation: Presentation, _ locale: Locale, _ digits: [Unicode.Scalar]) -> Affixes {
    let parts = split(format(1, code, presentation, locale), digits: digits)
    let fraction = parts.core.filter { digits.contains($0) }.count - 1
    return Affixes(fractionDigits: fraction, prefix: parts.left, suffix: parts.right)
}

// Mirror of `DecimalFormatStyleCurrency::format` in decimal_format_style_currency.rs. Keep the two identical.
func assemble(_ value: Decimal, _ data: LocaleData, _ affix: Affixes) -> String {
    var rounded = Decimal()
    var source = value
    NSDecimalRound(&rounded, &source, affix.fractionDigits, .bankers)
    let negative = value < 0
    let magnitude = rounded < 0 ? -rounded : rounded
    let plain = NSDecimalNumber(decimal: magnitude).stringValue
    let pieces = plain.split(separator: ".", omittingEmptySubsequences: false)
    let integer = Array(pieces[0])
    var fraction = pieces.count > 1 ? Array(pieces[1]) : []
    while fraction.count < affix.fractionDigits { fraction.append("0") }

    var groups: [[Character]] = []
    var rest = integer
    if rest.count >= data.primaryGrouping + data.minimumGroupingDigits {
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
    var number: [Unicode.Scalar] = []
    for (index, group) in groups.enumerated() {
        if index > 0 { number += data.groupingSeparator }
        number += localized(group)
    }
    if affix.fractionDigits > 0 {
        number += data.decimalSeparator
        number += localized(fraction)
    }

    var out: [Unicode.Scalar] = []
    if negative && data.minusBeforePrefix { out += data.minusSign }
    out += affix.prefix
    if negative && !data.minusBeforePrefix { out += data.minusSign }
    out += number
    out += affix.suffix
    var text = ""
    text.unicodeScalars.append(contentsOf: out)
    return text
}

func build(_ identifier: String, codes: [String]) -> LocaleData {
    let locale = Locale(identifier: identifier)
    let digits = localeDigits(locale)
    let formatter = NumberFormatter()
    formatter.locale = locale
    formatter.numberStyle = .currency
    let primary = formatter.groupingSize
    let secondary = formatter.secondaryGroupingSize > 0 ? formatter.secondaryGroupingSize : primary
    let decimal = Array((formatter.currencyDecimalSeparator ?? ".").unicodeScalars)
    let grouping = Array((formatter.currencyGroupingSeparator ?? ",").unicodeScalars)

    // Only the numeric core: a no-break space in the suffix is not a grouping separator.
    let probe = split(format(1234, "USD", .narrow, locale), digits: digits).core
    let minimumGrouping = probe.contains(where: { grouping.contains($0) }) ? 1 : 2

    let positive = affixes("USD", .narrow, locale, digits)
    let negativeParts = split(format(-1, "USD", .narrow, locale), digits: digits)
    if negativeParts.right != positive.suffix { fail("\(identifier): negative suffix differs") }
    let left = negativeParts.left
    let minus: [Unicode.Scalar]
    let beforePrefix: Bool
    if left.count >= positive.prefix.count && Array(left.suffix(positive.prefix.count)) == positive.prefix {
        minus = Array(left.dropLast(positive.prefix.count))
        beforePrefix = true
    } else if Array(left.prefix(positive.prefix.count)) == positive.prefix {
        minus = Array(left.dropFirst(positive.prefix.count))
        beforePrefix = false
    } else {
        fail("\(identifier): cannot place the minus sign in \(left)")
    }

    let template = (narrow: affixes(templateCode, .narrow, locale, digits),
                    standard: affixes(templateCode, .standard, locale, digits))
    var data = LocaleData(identifier: identifier, digits: digits, decimalSeparator: decimal,
                          groupingSeparator: grouping, primaryGrouping: primary,
                          secondaryGrouping: secondary, minimumGroupingDigits: minimumGrouping,
                          minusSign: minus, minusBeforePrefix: beforePrefix, template: template,
                          currencies: [])
    for code in codes {
        data.currencies.append((code, affixes(code, .narrow, locale, digits), affixes(code, .standard, locale, digits)))
    }
    return data
}

func substituted(_ affix: Affixes, _ code: String) -> Affixes {
    func swap(_ part: [Unicode.Scalar]) -> [Unicode.Scalar] {
        var text = ""
        text.unicodeScalars.append(contentsOf: part)
        return Array(text.replacingOccurrences(of: templateCode, with: code).unicodeScalars)
    }
    return Affixes(fractionDigits: affix.fractionDigits, prefix: swap(affix.prefix), suffix: swap(affix.suffix))
}

func verify(_ data: LocaleData) -> Int {
    let locale = Locale(identifier: data.identifier)
    var checked = 0
    func expect(_ value: Decimal, _ code: String, _ presentation: Presentation, _ affix: Affixes) {
        let want = format(value, code, presentation, locale)
        let got = assemble(value, data, affix)
        if want != got {
            fail("\(data.identifier) \(code) \(presentation) \(value): Foundation \(want.debugDescription), twin \(got.debugDescription)")
        }
        checked += 1
    }
    let values = samples.map { Decimal(string: $0, locale: Locale(identifier: "en_US_POSIX"))! }
    for value in values {
        for row in data.currencies {
            expect(value, row.code, .narrow, row.narrow)
            expect(value, row.code, .standard, row.standard)
        }
        for code in templateChecks {
            expect(value, code, .narrow, substituted(data.template.narrow, code))
            expect(value, code, .standard, substituted(data.template.standard, code))
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

func rustAffixes(_ affix: Affixes) -> String {
    "CurrencyAffixes { fraction_digits: \(affix.fractionDigits), prefix: \(rustString(affix.prefix)), suffix: \(rustString(affix.suffix)) }"
}

// Affix strings repeat across locales and currencies, so rows point into one interned table.
final class Interner {
    var strings: [[Unicode.Scalar]] = [[]]
    var index: [String: Int] = ["": 0]
    func id(_ scalars: [Unicode.Scalar]) -> Int {
        var key = ""
        key.unicodeScalars.append(contentsOf: scalars)
        if let found = index[key] { return found }
        strings.append(scalars)
        index[key] = strings.count - 1
        return strings.count - 1
    }
}

func render(_ all: [LocaleData]) -> String {
    let version = ProcessInfo.processInfo.operatingSystemVersionString
    let interner = Interner()
    var locales = ""
    for data in all {
        locales += "    LocaleCurrencyData {\n"
        locales += "        identifier: \"\(data.identifier)\",\n"
        locales += "        digits: [\(data.digits.map(rustChar).joined(separator: ", "))],\n"
        locales += "        decimal_separator: \(rustString(data.decimalSeparator)),\n"
        locales += "        grouping_separator: \(rustString(data.groupingSeparator)),\n"
        locales += "        primary_grouping: \(data.primaryGrouping),\n"
        locales += "        secondary_grouping: \(data.secondaryGrouping),\n"
        locales += "        minimum_grouping_digits: \(data.minimumGroupingDigits),\n"
        locales += "        minus_sign: \(rustString(data.minusSign)),\n"
        locales += "        minus_before_prefix: \(data.minusBeforePrefix),\n"
        locales += "        template_narrow: \(rustAffixes(data.template.narrow)),\n"
        locales += "        template_standard: \(rustAffixes(data.template.standard)),\n"
        locales += "        currencies: &["
        for (offset, row) in data.currencies.enumerated() {
            if row.narrow.fractionDigits != row.standard.fractionDigits {
                fail("\(data.identifier) \(row.code): narrow and standard fraction digits differ")
            }
            locales += offset % 6 == 0 ? "\n            " : " "
            locales += "row(\"\(row.code)\", \(row.narrow.fractionDigits), \(interner.id(row.narrow.prefix)), "
            locales += "\(interner.id(row.narrow.suffix)), \(interner.id(row.standard.prefix)), \(interner.id(row.standard.suffix))),"
        }
        locales += "\n        ],\n"
        locales += "    },\n"
    }
    var out = """
    // @generated by crates/rise-i18n/tools/gen_currency_format.swift from Foundation (macOS \(version)). Do not edit.
    #![cfg_attr(rustfmt, rustfmt_skip)]

    use super::decimal_format_style_currency::{CurrencyAffixes, LocaleCurrencyData, row};

    pub(super) const TEMPLATE_CODE: &str = "\(templateCode)";

    pub(super) static AFFIXES: &[&str] = &[

    """
    for (offset, scalars) in interner.strings.enumerated() {
        out += offset % 8 == 0 ? (offset == 0 ? "    " : "\n    ") : " "
        out += rustString(scalars) + ","
    }
    out += "\n];\n\npub(super) static LOCALES: &[LocaleCurrencyData] = &[\n"
    out += locales
    out += "];\n"
    return out
}

let arguments = Array(CommandLine.arguments.dropFirst())
if arguments.first == "--vectors" {
    guard arguments.count == 3 else { fail("--vectors <locale> <CODE,CODE>") }
    let locale = Locale(identifier: arguments[1])
    for code in arguments[2].split(separator: ",").map(String.init) {
        for sample in samples {
            let value = Decimal(string: sample, locale: Locale(identifier: "en_US_POSIX"))!
            print(arguments[1], code, sample, rustString(Array(format(value, code, .narrow, locale).unicodeScalars)),
                  rustString(Array(format(value, code, .standard, locale).unicodeScalars)))
        }
    }
    exit(0)
}

let codes = Locale.commonISOCurrencyCodes.sorted()
// The root locale ("") is what Foundation falls back to for an identifier it does not know.
let identifiers = [""] + catalogLocales()
var all: [LocaleData] = []
var checked = 0
for identifier in identifiers {
    let data = build(identifier, codes: codes)
    checked += verify(data)
    all.append(data)
}
let text = render(all)
print("locales \(all.count), currencies \(codes.count), checked \(checked) strings, \(text.utf8.count) bytes")
if arguments.first == "--check" { exit(0) }
try? FileManager.default.createDirectory(atPath: (outputPath as NSString).deletingLastPathComponent,
                                         withIntermediateDirectories: true)
do {
    try text.write(toFile: outputPath, atomically: true, encoding: .utf8)
} catch {
    fail("cannot write \(outputPath): \(error)")
}
print("wrote \(outputPath)")
