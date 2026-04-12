import java.util.regex.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

/**
 * Verifies JSONL regex test files against the real Java regex engine.
 *
 * Usage:
 *   javac VerifyTests.java
 *   java VerifyTests file1.jsonl [file2.jsonl ...]
 *
 * If no arguments given, verifies all *_tests*.jsonl files in the current
 * directory and in tests/.
 *
 * Each JSONL line is a JSON object with:
 *   id, pattern, input, op, expect, flags (optional), replacement (optional)
 *
 * Supported ops: matches, find, replaceAll, split, compile_error
 *
 * For "find" tests, only the "m" (matched text) field of each match object
 * is compared; extra fields like "groups"/"g" in expected are ignored.
 */
public class VerifyTests {

    // === JSON helpers ===

    static String jsonStr(String s) {
        if (s == null) return "null";
        StringBuilder sb = new StringBuilder("\"");
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"': sb.append("\\\""); break;
                case '\\': sb.append("\\\\"); break;
                case '\n': sb.append("\\n"); break;
                case '\r': sb.append("\\r"); break;
                case '\t': sb.append("\\t"); break;
                case '\b': sb.append("\\b"); break;
                case '\f': sb.append("\\f"); break;
                default:
                    if (c < 0x20) sb.append(String.format("\\u%04x", (int)c));
                    else sb.append(c);
            }
        }
        sb.append("\"");
        return sb.toString();
    }

    static String parseJsonString(String s) {
        if (s == null || s.equals("null")) return null;
        if (!s.startsWith("\"") || !s.endsWith("\"")) return s;
        s = s.substring(1, s.length() - 1);
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '\\' && i + 1 < s.length()) {
                char next = s.charAt(i + 1);
                switch (next) {
                    case '"': sb.append('"'); i++; break;
                    case '\\': sb.append('\\'); i++; break;
                    case 'n': sb.append('\n'); i++; break;
                    case 'r': sb.append('\r'); i++; break;
                    case 't': sb.append('\t'); i++; break;
                    case 'b': sb.append('\b'); i++; break;
                    case 'f': sb.append('\f'); i++; break;
                    case 'u':
                        if (i + 5 < s.length()) {
                            sb.append((char) Integer.parseInt(s.substring(i + 2, i + 6), 16));
                            i += 5;
                        }
                        break;
                    default: sb.append(c);
                }
            } else {
                sb.append(c);
            }
        }
        return sb.toString();
    }

    /** Extract a top-level JSON value for a key. */
    static String getJsonValue(String json, String key) {
        String search = "\"" + key + "\"";
        int idx = json.indexOf(search);
        if (idx < 0) return null;
        int start = idx + search.length();
        while (start < json.length() && (json.charAt(start) == ' ' || json.charAt(start) == ':'))
            start++;
        if (start >= json.length()) return null;
        return extractJsonValue(json, start);
    }

    /** Extract a JSON value starting at position start. Returns the raw JSON text. */
    static String extractJsonValue(String json, int start) {
        char first = json.charAt(start);
        if (first == '"') {
            int end = start + 1;
            while (end < json.length()) {
                if (json.charAt(end) == '\\') { end += 2; continue; }
                if (json.charAt(end) == '"') { end++; break; }
                end++;
            }
            return json.substring(start, end);
        } else if (first == '[' || first == '{') {
            char open = first, close = (first == '[') ? ']' : '}';
            int depth = 0, end = start;
            boolean inStr = false;
            while (end < json.length()) {
                char ch = json.charAt(end);
                if (inStr) {
                    if (ch == '\\') { end += 2; continue; }
                    if (ch == '"') inStr = false;
                } else {
                    if (ch == '"') inStr = true;
                    else if (ch == open) depth++;
                    else if (ch == close) { depth--; if (depth == 0) { end++; break; } }
                }
                end++;
            }
            return json.substring(start, end);
        } else if (first == 't') return "true";
        else if (first == 'f') return "false";
        else if (first == 'n') return "null";
        else {
            int end = start;
            while (end < json.length() && json.charAt(end) != ',' && json.charAt(end) != '}' && json.charAt(end) != ']')
                end++;
            return json.substring(start, end);
        }
    }

    /** Extract all "m" values from a find-result JSON array. */
    static List<String> extractMatchTexts(String jsonArray) {
        List<String> result = new ArrayList<>();
        if (jsonArray == null || !jsonArray.startsWith("[")) return result;
        // Find each {"m":...} object
        int i = 1; // skip '['
        while (i < jsonArray.length()) {
            char c = jsonArray.charAt(i);
            if (c == '{') {
                // Find "m" key in this object
                String mVal = getJsonValue(jsonArray.substring(i), "m");
                if (mVal != null) result.add(parseJsonString(mVal));
                // Skip to end of object
                int depth = 0;
                boolean inStr = false;
                while (i < jsonArray.length()) {
                    char ch = jsonArray.charAt(i);
                    if (inStr) {
                        if (ch == '\\') { i += 2; continue; }
                        if (ch == '"') inStr = false;
                    } else {
                        if (ch == '"') inStr = true;
                        else if (ch == '{') depth++;
                        else if (ch == '}') { depth--; if (depth == 0) { i++; break; } }
                    }
                    i++;
                }
            } else {
                i++;
            }
        }
        return result;
    }

    /** Extract string elements from a JSON array like ["a","b","c"]. */
    static List<String> extractStringArray(String jsonArray) {
        List<String> result = new ArrayList<>();
        if (jsonArray == null || !jsonArray.startsWith("[")) return result;
        int i = 1;
        while (i < jsonArray.length()) {
            while (i < jsonArray.length() && jsonArray.charAt(i) != '"' && jsonArray.charAt(i) != ']')
                i++;
            if (i >= jsonArray.length() || jsonArray.charAt(i) == ']') break;
            String val = extractJsonValue(jsonArray, i);
            result.add(parseJsonString(val));
            i += val.length();
        }
        return result;
    }

    // === Flag parsing ===

    static int buildFlags(String flagsStr) {
        int f = 0;
        if (flagsStr == null) return f;
        if (flagsStr.contains("i")) f |= Pattern.CASE_INSENSITIVE;
        if (flagsStr.contains("m")) f |= Pattern.MULTILINE;
        if (flagsStr.contains("s")) f |= Pattern.DOTALL;
        if (flagsStr.contains("x")) f |= Pattern.COMMENTS;
        if (flagsStr.contains("U")) f |= Pattern.UNICODE_CHARACTER_CLASS;
        if (flagsStr.contains("d")) f |= Pattern.UNIX_LINES;
        if (flagsStr.contains("u")) f |= Pattern.UNICODE_CASE;
        return f;
    }

    // === Verification ===

    static int[] verifyFile(String filename) throws Exception {
        List<String> lines = Files.readAllLines(Path.of(filename));
        int total = 0, correct = 0, wrong = 0;

        for (String line : lines) {
            line = line.trim();
            if (line.isEmpty()) continue;
            total++;

            String id = parseJsonString(getJsonValue(line, "id"));
            String pattern = parseJsonString(getJsonValue(line, "pattern"));
            String inputRaw = getJsonValue(line, "input");
            String input = inputRaw != null ? parseJsonString(inputRaw) : "";
            String op = parseJsonString(getJsonValue(line, "op"));
            String expectRaw = getJsonValue(line, "expect");
            String flagsRaw = getJsonValue(line, "flags");
            String flags = flagsRaw != null ? parseJsonString(flagsRaw) : "";
            String replacementRaw = getJsonValue(line, "replacement");
            String replacement = replacementRaw != null ? parseJsonString(replacementRaw) : "";

            if (op == null) {
                System.out.println("SKIP " + id + ": no op field");
                continue;
            }

            try {
                int f = buildFlags(flags);

                switch (op) {
                    case "compile_error": {
                        try {
                            Pattern.compile(pattern, f);
                            System.out.println("WRONG " + id + ": expected compile_error but compiled OK. pattern=" + jsonStr(pattern));
                            wrong++;
                        } catch (PatternSyntaxException e) {
                            correct++;
                        }
                        break;
                    }
                    case "matches": {
                        try {
                            boolean result = Pattern.compile(pattern, f).matcher(input).matches();
                            boolean expected = "true".equals(expectRaw);
                            if (result == expected) {
                                correct++;
                            } else {
                                System.out.println("WRONG " + id + ": matches(" + jsonStr(pattern) + ", " + jsonStr(input) + ") java=" + result + " expected=" + expected + " flags=" + jsonStr(flags));
                                wrong++;
                            }
                        } catch (PatternSyntaxException e) {
                            System.out.println("WRONG " + id + ": expected matches but got compile error: " + e.getDescription() + " pattern=" + jsonStr(pattern));
                            wrong++;
                        }
                        break;
                    }
                    case "find": {
                        try {
                            Matcher m = Pattern.compile(pattern, f).matcher(input);
                            List<String> actual = new ArrayList<>();
                            while (m.find()) actual.add(m.group());
                            List<String> expected = extractMatchTexts(expectRaw);
                            if (actual.equals(expected)) {
                                correct++;
                            } else {
                                System.out.println("WRONG " + id + ": find(" + jsonStr(pattern) + ", " + jsonStr(input) + ") java=" + actual + " expected=" + expected + " flags=" + jsonStr(flags));
                                wrong++;
                            }
                        } catch (PatternSyntaxException e) {
                            System.out.println("WRONG " + id + ": expected find but got compile error: " + e.getDescription() + " pattern=" + jsonStr(pattern));
                            wrong++;
                        }
                        break;
                    }
                    case "replaceAll": {
                        try {
                            String result = Pattern.compile(pattern, f).matcher(input).replaceAll(replacement);
                            String expected = parseJsonString(expectRaw);
                            if (result.equals(expected)) {
                                correct++;
                            } else {
                                System.out.println("WRONG " + id + ": replaceAll(" + jsonStr(pattern) + ", " + jsonStr(input) + ", " + jsonStr(replacement) + ") java=" + jsonStr(result) + " expected=" + jsonStr(expected));
                                wrong++;
                            }
                        } catch (Exception e) {
                            System.out.println("WRONG " + id + ": replaceAll exception: " + e.getMessage());
                            wrong++;
                        }
                        break;
                    }
                    case "split": {
                        try {
                            String[] parts = Pattern.compile(pattern, f).split(input);
                            List<String> actual = Arrays.asList(parts);
                            List<String> expected = extractStringArray(expectRaw);
                            if (actual.equals(expected)) {
                                correct++;
                            } else {
                                System.out.println("WRONG " + id + ": split(" + jsonStr(pattern) + ", " + jsonStr(input) + ") java=" + actual + " expected=" + expected);
                                wrong++;
                            }
                        } catch (PatternSyntaxException e) {
                            System.out.println("WRONG " + id + ": expected split but got compile error: " + e.getDescription());
                            wrong++;
                        }
                        break;
                    }
                    default:
                        System.out.println("SKIP " + id + ": unknown op " + op);
                }
            } catch (Exception e) {
                System.out.println("ERROR " + id + ": " + e.getMessage());
                wrong++;
            }
        }

        System.out.println("=== " + filename + " ===");
        System.out.println("Total: " + total + "  Correct: " + correct + "  WRONG: " + wrong);
        if (wrong == 0) System.out.println("ALL TESTS VERIFIED OK");
        System.out.println();
        return new int[]{total, correct, wrong};
    }

    public static void main(String[] args) throws Exception {
        List<String> files = new ArrayList<>();

        if (args.length > 0) {
            for (String arg : args) files.add(arg);
        } else {
            // Auto-discover JSONL test files
            for (String dir : new String[]{".", "tests"}) {
                File d = new File(dir);
                if (!d.isDirectory()) continue;
                for (File f : d.listFiles()) {
                    if (f.getName().endsWith(".jsonl") && f.getName().contains("test")) {
                        files.add(f.getPath());
                    }
                }
            }
            Collections.sort(files);
        }

        if (files.isEmpty()) {
            System.out.println("No JSONL test files found. Usage: java VerifyTests file1.jsonl [file2.jsonl ...]");
            System.exit(1);
        }

        int totalAll = 0, correctAll = 0, wrongAll = 0;
        for (String file : files) {
            int[] r = verifyFile(file);
            totalAll += r[0]; correctAll += r[1]; wrongAll += r[2];
        }

        System.out.println("========================================");
        System.out.println("GRAND TOTAL: " + totalAll + "  Correct: " + correctAll + "  WRONG: " + wrongAll);
        if (wrongAll == 0) {
            System.out.println("ALL TESTS ACROSS ALL FILES VERIFIED OK");
        } else {
            System.exit(1);
        }
    }
}
