use std::collections::HashSet;
use std::sync::LazyLock;

use gpui::{Hsla, rgb};
use rise_core::foundation::foundation_character::Character;
use rise_core::foundation::foundation_text::trimming_whitespaces;

use crate::rich_text::rich_text_markup::CharacterArray;
use crate::swiftui::rich_text::{AttributeContainer, AttributedString, RichFont};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CodePalette {
    pub background: Hsla,
    pub plain: Hsla,
    pub keyword: Hsla,
    pub r#type: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub comment: Hsla,
    pub function: Hsla,
    pub punctuation: Hsla,
    pub language_label: Hsla,
    pub chrome: Hsla,
}

impl CodePalette {
    pub fn editor_dark() -> Self {
        Self {
            background: rgb(0x171A21).into(),
            plain: rgb(0xE6E6EA).into(),
            keyword: rgb(0xC792EA).into(),
            r#type: rgb(0xFFCB6B).into(),
            string: rgb(0xC3E88D).into(),
            number: rgb(0xF78C6C).into(),
            comment: rgb(0x6B7394).into(),
            function: rgb(0x82AAFF).into(),
            punctuation: rgb(0x89DDFF).into(),
            language_label: rgb(0x8A90A6).into(),
            chrome: Hsla::from(rgb(0xFFFFFF)).opacity(0.18),
        }
    }
}

pub enum SyntaxHighlighter {}

impl SyntaxHighlighter {
    const PUNCTUATION: &'static str = "{}()[]<>.,;:=+-*/%&|^!~?";

    pub fn is_known(language: Option<&str>) -> bool {
        Self::spec(language).is_some()
    }

    pub fn display_name(language: Option<&str>) -> Option<String> {
        let language = language?;
        if trimming_whitespaces(language).is_empty() {
            return None;
        }
        Some(language.to_owned())
    }

    pub fn highlight(code: &str, language: Option<&str>, font: RichFont, palette: &CodePalette) -> AttributedString {
        let run_attributes = |color: Hsla| AttributeContainer {
            font: Some(font),
            foreground_color: Some(color),
            ..AttributeContainer::default()
        };

        let Some(spec) = Self::spec(language) else {
            return AttributedString::with_attributes(code, run_attributes(palette.plain));
        };

        let chars = CharacterArray::new(code);
        let n = chars.count();
        let mut out = AttributedString::default();
        let mut pending = String::new();

        let emit = |s: &str, color: Hsla, pending: &mut String, out: &mut AttributedString| {
            if !pending.is_empty() {
                out.append(AttributedString::with_attributes(
                    std::mem::take(pending),
                    run_attributes(palette.plain),
                ));
            }
            if s.is_empty() {
                return;
            }
            out.append(AttributedString::with_attributes(s, run_attributes(color)));
        };

        let mut i = 0;
        while i < n {
            let c = chars.at(i);

            if let Some((open, close)) = spec.block_comment
                && Self::matches(&chars, i, open)
            {
                let end = Self::index_past(&chars, i + Self::character_count(open), close, n).unwrap_or(n);
                emit(chars.slice(i, end), palette.comment, &mut pending, &mut out);
                i = end;
                continue;
            }

            if spec.line_comments.iter().any(|lc| Self::matches(&chars, i, lc)) {
                let mut j = i;
                while j < n && chars.at(j) != "\n" {
                    j += 1;
                }
                emit(chars.slice(i, j), palette.comment, &mut pending, &mut out);
                i = j;
                continue;
            }

            if spec.string_delimiters.contains(&c) {
                let end = Self::scan_string(&chars, i, c, n);
                emit(chars.slice(i, end), palette.string, &mut pending, &mut out);
                i = end;
                continue;
            }

            if Self::is_number(c) || (c == "." && i + 1 < n && Self::is_number(chars.at(i + 1))) {
                let end = Self::scan_number(&chars, i, n);
                emit(chars.slice(i, end), palette.number, &mut pending, &mut out);
                i = end;
                continue;
            }

            if Self::is_letter(c) || c == "_" || c == "$" || c == "@" || c == "#" {
                let mut j = i + 1;
                while j < n && Self::is_ident_char(chars.at(j)) {
                    j += 1;
                }
                let word = chars.slice(i, j);
                let color = Self::classify(word, spec, &chars, j, n, palette);
                emit(word, color, &mut pending, &mut out);
                i = j;
                continue;
            }

            if Self::is_punctuation(c) {
                emit(c, palette.punctuation, &mut pending, &mut out);
                i += 1;
                continue;
            }

            pending.push_str(c);
            i += 1;
        }
        emit("", palette.plain, &mut pending, &mut out);
        out
    }

    fn classify(
        word: &str,
        spec: &LanguageSpec,
        chars: &CharacterArray<'_>,
        end: usize,
        n: usize,
        palette: &CodePalette,
    ) -> Hsla {
        let probe = if spec.case_insensitive {
            word.to_uppercase()
        } else {
            word.to_owned()
        };
        if spec.keywords.contains(probe.as_str()) {
            return palette.keyword;
        }
        if spec.types.contains(word) {
            return palette.r#type;
        }
        let mut k = end;
        while k < n && (chars.at(k) == " " || chars.at(k) == "\t") {
            k += 1;
        }
        if k < n && chars.at(k) == "(" {
            return palette.function;
        }
        let first = CharacterArray::new(word);
        if first.count() > 0
            && let Some(first) = Character::new(first.at(0))
            && first.is_uppercase()
            && first.is_letter()
        {
            return palette.r#type;
        }
        palette.plain
    }

    fn is_number(c: &str) -> bool {
        Character::new(c).is_some_and(|c| c.is_number())
    }

    fn is_letter(c: &str) -> bool {
        Character::new(c).is_some_and(|c| c.is_letter())
    }

    fn is_ident_char(c: &str) -> bool {
        Self::is_letter(c) || Self::is_number(c) || c == "_" || c == "$"
    }

    fn is_punctuation(c: &str) -> bool {
        c.chars().count() == 1 && Self::PUNCTUATION.contains(c)
    }

    fn character_count(token: &str) -> usize {
        CharacterArray::new(token).count()
    }

    fn matches(chars: &CharacterArray<'_>, i: usize, token: &str) -> bool {
        let t = CharacterArray::new(token);
        if i + t.count() > chars.count() {
            return false;
        }
        (0..t.count()).all(|k| chars.at(i + k) == t.at(k))
    }

    fn index_past(chars: &CharacterArray<'_>, from: usize, terminator: &str, n: usize) -> Option<usize> {
        let count = Self::character_count(terminator);
        let mut i = from;
        while i < n {
            if Self::matches(chars, i, terminator) {
                return Some(i + count);
            }
            i += 1;
        }
        None
    }

    fn scan_string(chars: &CharacterArray<'_>, from: usize, delimiter: &str, n: usize) -> usize {
        let mut i = from + 1;
        while i < n {
            let c = chars.at(i);
            if c == "\\" {
                i += 2;
                continue;
            }
            if c == "\n" {
                return i;
            }
            if c == delimiter {
                return i + 1;
            }
            i += 1;
        }
        n
    }

    fn scan_number(chars: &CharacterArray<'_>, from: usize, n: usize) -> usize {
        let mut i = from;
        if chars.at(i) == "0" && i + 1 < n && matches!(chars.at(i + 1), "x" | "X" | "b") {
            i += 2;
        }
        while i < n {
            let c = chars.at(i);
            if Self::is_number(c) || Self::is_letter(c) || c == "." || c == "_" {
                i += 1;
            } else {
                break;
            }
        }
        i
    }

    fn spec(language: Option<&str>) -> Option<&'static LanguageSpec> {
        let raw = trimming_whitespaces(language?).to_lowercase();
        if raw.is_empty() {
            return None;
        }
        let spec: &'static LanguageSpec = match raw.as_str() {
            "swift" => &SWIFT_SPEC,
            "kotlin" | "kt" | "kts" => &KOTLIN_SPEC,
            "java" => &JAVA_SPEC,
            "javascript" | "js" | "jsx" | "node" => &JS_SPEC,
            "typescript" | "ts" | "tsx" => &TS_SPEC,
            "python" | "py" | "py3" => &PYTHON_SPEC,
            "c" | "h" => &C_SPEC,
            "cpp" | "c++" | "cc" | "cxx" | "hpp" => &CPP_SPEC,
            "csharp" | "c#" | "cs" => &CSHARP_SPEC,
            "objc" | "objective-c" | "obj-c" | "objectivec" | "m" | "mm" => &OBJC_SPEC,
            "go" | "golang" => &GO_SPEC,
            "rust" | "rs" => &RUST_SPEC,
            "ruby" | "rb" => &RUBY_SPEC,
            "php" => &PHP_SPEC,
            "dart" => &DART_SPEC,
            "scala" => &SCALA_SPEC,
            "sql" | "mysql" | "postgres" | "postgresql" | "sqlite" => &SQL_SPEC,
            "bash" | "sh" | "shell" | "zsh" | "shellscript" => &BASH_SPEC,
            "json" | "jsonc" | "json5" => &JSON_SPEC,
            "yaml" | "yml" => &YAML_SPEC,
            "html" | "xml" | "xhtml" | "svg" => &HTML_SPEC,
            "css" | "scss" | "sass" | "less" => &CSS_SPEC,
            _ => return None,
        };
        Some(spec)
    }
}

#[derive(Clone)]
struct LanguageSpec {
    keywords: HashSet<&'static str>,
    types: HashSet<&'static str>,
    line_comments: Vec<&'static str>,
    block_comment: Option<(&'static str, &'static str)>,
    string_delimiters: Vec<&'static str>,
    case_insensitive: bool,
}

impl LanguageSpec {
    /// defaults are line and block comments plus double and single quotes
    fn new(keywords: &[&'static str]) -> Self {
        Self {
            keywords: keywords.iter().copied().collect(),
            types: HashSet::new(),
            line_comments: vec!["//"],
            block_comment: Some(("/*", "*/")),
            string_delimiters: vec!["\"", "'"],
            case_insensitive: false,
        }
    }

    fn types(mut self, types: &[&'static str]) -> Self {
        self.types = types.iter().copied().collect();
        self
    }

    fn line_comments(mut self, line_comments: &[&'static str]) -> Self {
        self.line_comments = line_comments.to_vec();
        self
    }

    fn block_comment(mut self, block_comment: Option<(&'static str, &'static str)>) -> Self {
        self.block_comment = block_comment;
        self
    }

    fn string_delimiters(mut self, string_delimiters: &[&'static str]) -> Self {
        self.string_delimiters = string_delimiters.to_vec();
        self
    }

    fn case_insensitive(mut self, case_insensitive: bool) -> Self {
        self.case_insensitive = case_insensitive;
        self
    }
}

static SWIFT_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "func", "let", "var", "if", "else", "guard", "return", "for", "while", "in", "switch", "case", "default",
        "break", "continue", "struct", "class", "enum", "protocol", "extension", "import", "init", "deinit", "self",
        "Self", "super", "nil", "true", "false", "public", "private", "internal", "fileprivate", "open", "static",
        "final", "lazy", "weak", "unowned", "throws", "rethrows", "try", "catch", "throw", "async", "await", "actor",
        "where", "as", "is", "some", "any", "associatedtype", "typealias", "defer", "do", "repeat", "fallthrough",
        "mutating", "nonmutating", "override", "convenience", "required", "indirect", "subscript", "operator",
        "inout", "@escaping", "@autoclosure", "@objc", "@MainActor", "@State", "@Binding", "@Observable",
        "@Environment",
    ])
    .types(&[
        "Int", "Double", "Float", "String", "Bool", "Character", "Array", "Dictionary", "Set", "Optional", "Void",
        "Any", "AnyObject", "Result", "Error", "Data", "Date", "URL", "UUID", "CGFloat", "CGRect", "CGSize",
        "CGPoint", "View", "Color", "Text", "Image",
    ])
});

static KOTLIN_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "fun", "val", "var", "if", "else", "when", "for", "while", "do", "return", "break", "continue", "class",
        "object", "interface", "enum", "data", "sealed", "abstract", "open", "override", "private", "public",
        "protected", "internal", "import", "package", "this", "super", "null", "true", "false", "is", "as", "in",
        "out", "by", "lateinit", "companion", "init", "constructor", "suspend", "inline", "reified", "vararg",
        "typealias", "try", "catch", "finally", "throw",
    ])
    .types(&[
        "Int", "Long", "Double", "Float", "String", "Boolean", "Char", "Byte", "Short", "Unit", "Any", "List", "Map",
        "Set", "Array", "Nothing",
    ])
});

static JAVA_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "public", "private", "protected", "class", "interface", "enum", "extends", "implements", "static", "final",
        "abstract", "void", "new", "return", "if", "else", "for", "while", "do", "switch", "case", "default",
        "break", "continue", "this", "super", "null", "true", "false", "import", "package", "try", "catch",
        "finally", "throw", "throws", "synchronized", "volatile", "transient", "instanceof", "native",
    ])
    .types(&[
        "int", "long", "double", "float", "boolean", "char", "byte", "short", "String", "Object", "Integer",
        "Double", "Boolean", "List", "Map", "Set", "Array",
    ])
});

static JS_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "function", "const", "let", "var", "if", "else", "for", "while", "do", "return", "break", "continue",
        "switch", "case", "default", "class", "extends", "new", "this", "super", "null", "undefined", "true",
        "false", "typeof", "instanceof", "in", "of", "import", "export", "from", "as", "async", "await", "yield",
        "try", "catch", "finally", "throw", "delete", "void", "static", "get", "set",
    ])
    .types(&[
        "String", "Number", "Boolean", "Object", "Array", "Promise", "Map", "Set", "Symbol", "JSON", "Math",
        "console",
    ])
    .string_delimiters(&["\"", "'", "`"])
});

static TS_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    let mut spec = JS_SPEC.clone();
    spec.keywords.extend([
        "interface", "type", "enum", "namespace", "declare", "implements", "private", "public", "protected",
        "readonly", "abstract", "keyof", "infer", "is", "satisfies", "override",
    ]);
    spec.types.extend([
        "string", "number", "boolean", "any", "unknown", "never", "void", "Record", "Partial", "Readonly",
        "ReturnType",
    ]);
    spec
});

static PYTHON_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "def", "class", "return", "if", "elif", "else", "for", "while", "break", "continue", "pass", "import",
        "from", "as", "with", "try", "except", "finally", "raise", "lambda", "yield", "global", "nonlocal", "del",
        "assert", "and", "or", "not", "in", "is", "None", "True", "False", "async", "await", "self", "cls", "match",
        "case",
    ])
    .types(&[
        "int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object", "type", "range",
    ])
    .line_comments(&["#"])
    .block_comment(None)
    .string_delimiters(&["\"", "'"])
});

static C_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "int", "char", "float", "double", "void", "long", "short", "signed", "unsigned", "struct", "union", "enum",
        "typedef", "static", "extern", "const", "volatile", "register", "return", "if", "else", "for", "while",
        "do", "switch", "case", "default", "break", "continue", "goto", "sizeof", "NULL", "true", "false",
        "#include", "#define", "#ifdef", "#ifndef", "#endif", "#pragma",
    ])
    .types(&[
        "size_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint32_t", "uint64_t", "bool", "FILE",
    ])
});

static CPP_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    let mut spec = C_SPEC.clone();
    spec.keywords.extend([
        "class", "public", "private", "protected", "virtual", "override", "namespace", "using", "template",
        "typename", "new", "delete", "this", "nullptr", "try", "catch", "throw", "operator", "friend", "inline",
        "explicit", "constexpr", "auto", "decltype", "noexcept", "mutable",
    ]);
    spec.types.extend([
        "string", "vector", "map", "set", "pair", "unordered_map", "shared_ptr", "unique_ptr",
    ]);
    spec
});

static CSHARP_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "public", "private", "protected", "internal", "class", "interface", "struct", "enum", "namespace", "using",
        "static", "readonly", "const", "void", "new", "return", "if", "else", "for", "foreach", "while", "do",
        "switch", "case", "default", "break", "continue", "this", "base", "null", "true", "false", "var", "async",
        "await", "try", "catch", "finally", "throw", "get", "set", "override", "virtual", "abstract", "sealed",
        "partial", "is", "as", "in", "out", "ref", "params",
    ])
    .types(&[
        "int", "long", "double", "float", "bool", "char", "byte", "string", "object", "decimal", "List",
        "Dictionary", "Task", "var",
    ])
});

static OBJC_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "@interface", "@implementation", "@end", "@property", "@synthesize", "@protocol", "@selector", "@class",
        "@autoreleasepool", "self", "super", "nil", "YES", "NO", "if", "else", "for", "while", "do", "switch",
        "case", "default", "break", "continue", "return", "void", "id", "instancetype", "const", "static",
        "typedef", "struct", "enum", "#import", "#include", "#define",
    ])
    .types(&[
        "NSString", "NSArray", "NSDictionary", "NSObject", "NSInteger", "NSUInteger", "BOOL", "CGFloat", "CGRect",
        "int", "char", "float", "double",
    ])
});

static GO_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "func", "var", "const", "type", "struct", "interface", "map", "chan", "package", "import", "return", "if",
        "else", "for", "range", "switch", "case", "default", "break", "continue", "go", "defer", "select", "nil",
        "true", "false", "iota", "fallthrough", "goto",
    ])
    .types(&[
        "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint32", "uint64", "float32", "float64",
        "string", "bool", "byte", "rune", "error", "any",
    ])
    .string_delimiters(&["\"", "'", "`"])
});

static RUST_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "fn", "let", "mut", "const", "static", "if", "else", "match", "for", "while", "loop", "return", "break",
        "continue", "struct", "enum", "trait", "impl", "mod", "use", "pub", "crate", "self", "Self", "super",
        "where", "as", "ref", "move", "dyn", "async", "await", "unsafe", "extern", "type", "true", "false", "None",
        "Some", "Ok", "Err",
    ])
    .types(&[
        "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "isize", "f32", "f64", "bool", "char", "str",
        "String", "Vec", "Option", "Result", "Box", "Rc", "Arc",
    ])
});

static RUBY_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "def", "end", "class", "module", "if", "elsif", "else", "unless", "case", "when", "while", "until", "for",
        "do", "return", "yield", "begin", "rescue", "ensure", "raise", "require", "require_relative",
        "attr_accessor", "attr_reader", "attr_writer", "self", "nil", "true", "false", "and", "or", "not", "then",
        "lambda", "proc", "puts", "new",
    ])
    .line_comments(&["#"])
    .block_comment(None)
    .string_delimiters(&["\"", "'"])
});

static PHP_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "function", "class", "interface", "trait", "extends", "implements", "public", "private", "protected",
        "static", "const", "abstract", "final", "return", "if", "else", "elseif", "for", "foreach", "while", "do",
        "switch", "case", "default", "break", "continue", "new", "this", "self", "parent", "null", "true", "false",
        "echo", "print", "use", "namespace", "try", "catch", "finally", "throw", "as", "instanceof",
    ])
    .line_comments(&["//", "#"])
    .string_delimiters(&["\"", "'"])
});

static DART_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "void", "var", "final", "const", "class", "extends", "implements", "with", "abstract", "enum", "mixin",
        "if", "else", "for", "while", "do", "switch", "case", "default", "break", "continue", "return", "new",
        "this", "super", "null", "true", "false", "async", "await", "yield", "import", "export", "library", "part",
        "try", "catch", "finally", "throw", "rethrow", "is", "as", "get", "set", "factory", "late", "required",
    ])
    .types(&[
        "int", "double", "num", "String", "bool", "List", "Map", "Set", "Object", "dynamic", "Future", "Stream",
        "Widget",
    ])
});

static SCALA_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "def", "val", "var", "class", "object", "trait", "extends", "with", "if", "else", "for", "while", "do",
        "match", "case", "return", "new", "this", "super", "null", "true", "false", "import", "package", "private",
        "protected", "override", "abstract", "final", "sealed", "implicit", "lazy", "type", "yield", "try",
        "catch", "finally", "throw",
    ])
    .types(&[
        "Int", "Long", "Double", "Float", "String", "Boolean", "Char", "Unit", "Any", "AnyRef", "Option", "List",
        "Map", "Set", "Seq", "Array",
    ])
});

static SQL_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE",
        "ALTER", "DROP", "INDEX", "VIEW", "JOIN", "INNER", "LEFT", "RIGHT", "OUTER", "FULL", "ON", "AS", "AND", "OR",
        "NOT", "NULL", "IS", "IN", "LIKE", "BETWEEN", "ORDER", "BY", "GROUP", "HAVING", "LIMIT", "OFFSET",
        "DISTINCT", "COUNT", "SUM", "AVG", "MIN", "MAX", "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "DEFAULT",
        "UNIQUE", "CONSTRAINT", "CASE", "WHEN", "THEN", "ELSE", "END", "UNION", "ALL", "TRUE", "FALSE",
    ])
    .types(&[
        "INT", "INTEGER", "BIGINT", "SMALLINT", "VARCHAR", "CHAR", "TEXT", "BOOLEAN", "DATE", "TIMESTAMP", "FLOAT",
        "DOUBLE", "DECIMAL", "NUMERIC", "SERIAL", "UUID", "JSON", "JSONB",
    ])
    .line_comments(&["--"])
    .string_delimiters(&["'", "\""])
    .case_insensitive(true)
});

static BASH_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "function",
        "return", "in", "select", "echo", "export", "local", "readonly", "source", "alias", "unset", "exit", "cd",
        "set", "trap", "true", "false",
    ])
    .line_comments(&["#"])
    .block_comment(None)
    .string_delimiters(&["\"", "'", "`"])
});

static JSON_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&["true", "false", "null"])
        .line_comments(&["//"])
        .string_delimiters(&["\""])
});

static YAML_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&["true", "false", "null", "yes", "no", "on", "off"])
        .line_comments(&["#"])
        .block_comment(None)
        .string_delimiters(&["\"", "'"])
});

static HTML_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "html", "head", "body", "div", "span", "a", "p", "img", "ul", "ol", "li", "table", "tr", "td", "th",
        "script", "style", "link", "meta", "title", "header", "footer", "nav", "section", "article", "input",
        "button", "form", "label",
    ])
    .line_comments(&[])
    .block_comment(Some(("<!--", "-->")))
    .string_delimiters(&["\"", "'"])
});

static CSS_SPEC: LazyLock<LanguageSpec> = LazyLock::new(|| {
    LanguageSpec::new(&[
        "important", "inherit", "initial", "none", "auto", "flex", "grid", "block", "inline", "absolute",
        "relative", "fixed", "hidden", "visible", "solid", "dashed", "bold", "italic", "center", "left", "right",
    ])
    .line_comments(&["//"])
    .block_comment(Some(("/*", "*/")))
    .string_delimiters(&["\"", "'"])
});
