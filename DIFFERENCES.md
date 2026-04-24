# Differences from Java's java.util.regex

This document describes the known behavioral differences between this Rust implementation and Java's `java.util.regex.Pattern` engine (tested against OpenJDK 25).

## 1. Group captures leaking across find() start positions

When Java's `Matcher.find()` tries a pattern at a given start position and the overall match fails, group captures set during that failed attempt can persist into the match that eventually succeeds at a later position.

Example:

```
Pattern: (?=(\w))*\s
Input:   "a "
```

Java returns a match at position 1-2 (the space) with group 1 = `"a"`. The group was captured when the engine tried position 0: the lookahead `(?=(\w))` matched `a`, but then `\s` failed on `a`, so the engine moved to position 1. At position 1, the lookahead fails (space is not `\w`), so zero iterations, and `\s` matches. Java retains `g1="a"` from the failed attempt at position 0.

This Rust implementation resets all group captures when starting a new position, so group 1 is `None`. This affects `find()`, `replaceAll()`, and `split()` results when replacements or downstream logic references group values (`$1`, `$2`, etc.). Match text and match positions are always identical to Java.

This behavior is widely considered an unintended quirk of the JDK implementation. Other regex engines (Perl, .NET, Python) reset group captures between attempts, matching the behavior of this implementation.

## 2. Lookbehind with unbounded group quantifiers accepted

Java rejects lookbehind patterns where it cannot compute a finite maximum length for a group quantifier:

```
Pattern: (?<=(?:ab)+)c
Java:    PatternSyntaxException ("Look-behind group does not have an obvious maximum length")
Rust:    Compiles and matches correctly
```

Java does accept unbounded quantifiers on single characters (e.g., `(?<=a+)` is allowed), but not on groups like `(?:ab)+`.

This implementation accepts all lookbehind patterns and handles them correctly at runtime by trying all possible start positions. This is strictly more permissive than Java. Patterns that Java rejects will work here, but code relying on the compile-time error will see a difference.

## 3. Group captures inside negative lookbehinds after failed attempts

In Java, groups inside a negative lookbehind can retain values from a failed internal attempt:

```
Pattern: (?<!(a|bb))c
Input:   "ac"
```

The negative lookbehind `(?<!(a|bb))` checks if `a` or `bb` precedes position 1. The branch `(a)` matches at position 0, setting group 1 = `"a"`. Since the lookbehind content matched, the negative assertion fails, and `c` is not matched at position 1. However, Java may retain the group 1 capture from that internal attempt in subsequent matching.

This implementation properly resets group captures from failed lookbehind attempts. This is a very niche scenario that only manifests when groups inside negative lookbehinds are referenced elsewhere in the pattern or in replacements.

## Summary

| Behavior | Java | This implementation |
|---|---|---|
| Group capture reset between find positions | No (leaks) | Yes (clean reset) |
| `(?<=(?:ab)+)` lookbehind | Compile error | Accepted and works |
| Negative lookbehind group capture leak | Leaks in some cases | Clean reset |

All three differences involve edge cases in group capture state management. For all tested patterns, the match text, match positions, split boundaries, and replacement results (with literal replacements) are identical to Java across 200,000+ random differential tests.
