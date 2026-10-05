/* Copyright (c) 2026 tabnas, MIT License */

// JSON text: the escaping and number spelling every runtime shares.

// `s` as a JSON string literal, escaped as RFC 8259 requires: `"` and `\`
// escaped, the short forms `\b \f \n \r \t`, other control characters as
// `\u00XX` with lowercase hex, and everything else as itself.
export function jsonString(s: string): string {
  let out = '"'
  let from = 0
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (c >= 0x20 && c !== 0x22 && c !== 0x5c) continue
    if (i > from) out += s.slice(from, i)
    from = i + 1
    switch (c) {
      case 0x22:
        out += '\\"'
        break
      case 0x5c:
        out += '\\\\'
        break
      case 0x08:
        out += '\\b'
        break
      case 0x0c:
        out += '\\f'
        break
      case 0x0a:
        out += '\\n'
        break
      case 0x0d:
        out += '\\r'
        break
      case 0x09:
        out += '\\t'
        break
      default:
        out += '\\u' + c.toString(16).padStart(4, '0')
    }
  }
  if (from < s.length) out += s.slice(from)
  return out + '"'
}

// A number with no lexeme, as text: the shortest digits that read back as
// the same double, written positionally, never with an exponent (`1e21` is
// `1000000000000000000000`, `1e-7` is `0.0000001`, `-0` is `-0`). This is
// the spelling the Rust runtime's `f64` `Display` gives, so a container
// cell holding such a number reads the same in every runtime;
// `DIVERGENCE.md` keeps the decision open. A non-finite value has no JSON
// form and is `null`.
export function numberText(value: number): string {
  if (!Number.isFinite(value)) return 'null'
  if (Object.is(value, -0)) return '-0'
  if (0 === value) return '0'
  const neg = value < 0
  const exp = Math.abs(value).toExponential()
  const at = exp.indexOf('e')
  const digits = exp.slice(0, at).replace('.', '')
  const e = parseInt(exp.slice(at + 1), 10)
  let text: string
  if (e >= digits.length - 1) {
    text = digits + '0'.repeat(e - (digits.length - 1))
  } else if (e >= 0) {
    text = digits.slice(0, e + 1) + '.' + digits.slice(e + 1)
  } else {
    text = '0.' + '0'.repeat(-e - 1) + digits
  }
  return neg ? '-' + text : text
}

// A number as JSON text: its lexeme when known, else `numberText`.
export function jsonNumber(value: number, lexeme: string | null | undefined): string {
  return null != lexeme ? lexeme : numberText(value)
}

// RFC 8259's number grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
// Only such a lexeme is kept, so a renderer can write it as it stands.
export function isJsonNumber(s: string): boolean {
  return /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$/.test(s)
}
