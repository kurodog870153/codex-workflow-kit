//! Unified text diffs for reviewed Specification migration previews.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tag {
    Equal,
    Replace,
    Delete,
    Insert,
}

#[derive(Clone, Copy)]
struct Opcode {
    tag: Tag,
    a1: usize,
    a2: usize,
    b1: usize,
    b2: usize,
}

fn lines(raw: &[u8]) -> Vec<String> {
    match std::str::from_utf8(raw) {
        Ok(text) => {
            let mut result = Vec::new();
            let mut start = 0;
            let mut characters = text.char_indices().peekable();
            while let Some((position, character)) = characters.next() {
                let separator = matches!(
                    character,
                    '\n' | '\r'
                        | '\u{000b}'
                        | '\u{000c}'
                        | '\u{001c}'
                        | '\u{001d}'
                        | '\u{001e}'
                        | '\u{0085}'
                        | '\u{2028}'
                        | '\u{2029}'
                );
                if separator {
                    let mut end = position + character.len_utf8();
                    if character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n')
                    {
                        let (next_position, next) = characters.next().expect("peeked newline");
                        end = next_position + next.len_utf8();
                    }
                    result.push(text[start..end].to_owned());
                    start = end;
                }
            }
            if start < text.len() {
                result.push(text[start..].to_owned());
            }
            result
        }
        Err(_) => vec![format!(
            "<binary sha256={}>\n",
            work_operations::derivation::fingerprint::raw(raw)
        )],
    }
}

fn range(start: usize, length: usize) -> String {
    match length {
        0 => format!("{start},0"),
        1 => (start + 1).to_string(),
        _ => format!("{},{}", start + 1, length),
    }
}

fn longest_match(
    old: &[String],
    new: &[String],
    positions: &HashMap<&str, Vec<usize>>,
    bounds: (usize, usize, usize, usize),
) -> (usize, usize, usize) {
    let (alo, ahi, blo, bhi) = bounds;
    let (mut best_a, mut best_b, mut best_size) = (alo, blo, 0);
    let mut previous = HashMap::<usize, usize>::new();
    for (a, line) in old.iter().enumerate().take(ahi).skip(alo) {
        let mut current = HashMap::new();
        if let Some(matches) = positions.get(line.as_str()) {
            for &b in matches.iter().filter(|&&b| b >= blo && b < bhi) {
                let size = if b == 0 {
                    1
                } else {
                    previous.get(&(b - 1)).copied().unwrap_or(0) + 1
                };
                current.insert(b, size);
                if size > best_size {
                    (best_a, best_b, best_size) = (a + 1 - size, b + 1 - size, size);
                }
            }
        }
        previous = current;
    }
    // Matching extends popular lines after finding the longest anchor.
    while best_a > alo && best_b > blo && old[best_a - 1] == new[best_b - 1] {
        best_a -= 1;
        best_b -= 1;
        best_size += 1;
    }
    while best_a + best_size < ahi
        && best_b + best_size < bhi
        && old[best_a + best_size] == new[best_b + best_size]
    {
        best_size += 1;
    }
    (best_a, best_b, best_size)
}

fn opcodes(old: &[String], new: &[String]) -> Vec<Opcode> {
    let mut positions = HashMap::<&str, Vec<usize>>::new();
    for (position, line) in new.iter().enumerate() {
        positions.entry(line.as_str()).or_default().push(position);
    }
    if new.len() >= 200 {
        let threshold = new.len() / 100 + 1;
        positions.retain(|_, indices| indices.len() <= threshold);
    }
    let mut queue = vec![(0, old.len(), 0, new.len())];
    let mut matches = Vec::new();
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (a, b, size) = longest_match(old, new, &positions, (alo, ahi, blo, bhi));
        if size > 0 {
            matches.push((a, b, size));
            if alo < a && blo < b {
                queue.push((alo, a, blo, b));
            }
            if a + size < ahi && b + size < bhi {
                queue.push((a + size, ahi, b + size, bhi));
            }
        }
    }
    matches.sort_unstable();
    let mut merged: Vec<(usize, usize, usize)> = Vec::new();
    for (a, b, size) in matches {
        if let Some(last) = merged.last_mut() {
            if last.0 + last.2 == a && last.1 + last.2 == b {
                last.2 += size;
                continue;
            }
        }
        merged.push((a, b, size));
    }
    merged.push((old.len(), new.len(), 0));
    let (mut a, mut b) = (0, 0);
    let mut codes = Vec::new();
    for (next_a, next_b, size) in merged {
        let tag = if a < next_a && b < next_b {
            Some(Tag::Replace)
        } else if a < next_a {
            Some(Tag::Delete)
        } else if b < next_b {
            Some(Tag::Insert)
        } else {
            None
        };
        if let Some(tag) = tag {
            codes.push(Opcode {
                tag,
                a1: a,
                a2: next_a,
                b1: b,
                b2: next_b,
            });
        }
        a = next_a + size;
        b = next_b + size;
        if size > 0 {
            codes.push(Opcode {
                tag: Tag::Equal,
                a1: next_a,
                a2: a,
                b1: next_b,
                b2: b,
            });
        }
    }
    codes
}

fn grouped_opcodes(mut codes: Vec<Opcode>) -> Vec<Vec<Opcode>> {
    const CONTEXT: usize = 3;
    if codes.is_empty() {
        codes.push(Opcode {
            tag: Tag::Equal,
            a1: 0,
            a2: 1,
            b1: 0,
            b2: 1,
        });
    }
    if codes[0].tag == Tag::Equal {
        codes[0].a1 = codes[0].a1.max(codes[0].a2.saturating_sub(CONTEXT));
        codes[0].b1 = codes[0].b1.max(codes[0].b2.saturating_sub(CONTEXT));
    }
    let last = codes.len() - 1;
    if codes[last].tag == Tag::Equal {
        codes[last].a2 = codes[last].a2.min(codes[last].a1 + CONTEXT);
        codes[last].b2 = codes[last].b2.min(codes[last].b1 + CONTEXT);
    }
    let mut groups = Vec::new();
    let mut group = Vec::new();
    for mut code in codes {
        if code.tag == Tag::Equal && code.a2 - code.a1 > 2 * CONTEXT {
            group.push(Opcode {
                a2: code.a1 + CONTEXT,
                b2: code.b1 + CONTEXT,
                ..code
            });
            groups.push(group);
            group = Vec::new();
            code.a1 = code.a2 - CONTEXT;
            code.b1 = code.b2 - CONTEXT;
        }
        group.push(code);
    }
    if !(group.is_empty() || group.len() == 1 && group[0].tag == Tag::Equal) {
        groups.push(group);
    }
    groups
}

pub fn unified_diff(path: &str, before: Option<&[u8]>, after: Option<&[u8]>) -> String {
    let old = before.map(lines).unwrap_or_default();
    let new = after.map(lines).unwrap_or_default();
    if old == new {
        return String::new();
    }
    let mut output = format!("--- {path}:source\n+++ {path}:candidate\n");
    for group in grouped_opcodes(opcodes(&old, &new)) {
        let first = group.first().expect("nonempty group");
        let last = group.last().expect("nonempty group");
        output.push_str(&format!(
            "@@ -{} +{} @@\n",
            range(first.a1, last.a2 - first.a1),
            range(first.b1, last.b2 - first.b1)
        ));
        for code in group {
            if matches!(code.tag, Tag::Equal | Tag::Replace | Tag::Delete) {
                for line in &old[code.a1..code.a2] {
                    output.push(if code.tag == Tag::Equal { ' ' } else { '-' });
                    output.push_str(line);
                }
            }
            if matches!(code.tag, Tag::Replace | Tag::Insert) {
                for line in &new[code.b1..code.b2] {
                    output.push('+');
                    output.push_str(line);
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::unified_diff;

    #[test]
    fn repeated_first_new_line_does_not_extend_previous_match() {
        assert_eq!(
            unified_diff("sample", Some(b"x\nx\n"), Some(b"x\n")),
            "--- sample:source\n+++ sample:candidate\n@@ -1,2 +1 @@\n x\n-x\n"
        );
    }

    #[test]
    fn legacy_crlf_and_unicode_line_breaks_keep_current_contract_diff_lines() {
        assert_eq!(
            unified_diff("sample", Some(b"a\r\nb\r\n"), Some(b"a\r\nc\r\n")),
            "--- sample:source\n+++ sample:candidate\n@@ -1,2 +1,2 @@\n a\r\n-b\r\n+c\r\n"
        );
        assert_eq!(
            unified_diff(
                "sample",
                Some("a\u{2028}b".as_bytes()),
                Some("a\u{2028}c".as_bytes())
            ),
            "--- sample:source\n+++ sample:candidate\n@@ -1,2 +1,2 @@\n a\u{2028}-b+c"
        );
    }
}
