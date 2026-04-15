import java.util.regex.*;
import java.io.*;
import java.util.*;

/**
 * Differential testing: generates random regex patterns and inputs,
 * runs them through Java's regex engine, outputs JSONL for comparison
 * with the Rust implementation.
 */
public class DiffTest {
    static Random rng;
    static int id = 0;
    static PrintWriter out;
    static int generated = 0;
    static int skipped = 0;

    static String esc(String s) {
        StringBuilder sb = new StringBuilder();
        for (char c : s.toCharArray()) {
            if (c == '"') sb.append("\\\"");
            else if (c == '\\') sb.append("\\\\");
            else if (c == '\n') sb.append("\\n");
            else if (c == '\r') sb.append("\\r");
            else if (c == '\t') sb.append("\\t");
            else if (c < 0x20) sb.append(String.format("\\u%04x", (int)c));
            else sb.append(c);
        }
        return sb.toString();
    }

    // === Pattern generators ===

    static String randomChar() {
        String chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        return "" + chars.charAt(rng.nextInt(chars.length()));
    }

    static String randomAtom() {
        int r = rng.nextInt(20);
        if (r < 5) return randomChar();
        if (r < 7) return ".";
        if (r < 8) return "\\d";
        if (r < 9) return "\\w";
        if (r < 10) return "\\s";
        if (r < 11) return "\\D";
        if (r < 12) return "\\W";
        if (r < 13) return "\\S";
        if (r < 14) return randomCharClass();
        if (r < 15) return "\\b";
        if (r < 16) return randomEscapedLiteral();
        if (r < 17) return "\\p{" + randomUnicodeProperty() + "}";
        if (r < 18) return "(" + randomPattern(rng.nextInt(2) + 1) + ")";
        if (r < 19) return "(?:" + randomPattern(rng.nextInt(2) + 1) + ")";
        return randomChar();
    }

    static String randomEscapedLiteral() {
        String specials = ".+*?^$|\\()[]{}";
        return "\\" + specials.charAt(rng.nextInt(specials.length()));
    }

    static String randomUnicodeProperty() {
        String[] props = {"L", "Lu", "Ll", "Lt", "Lm", "Lo", "M", "Mn", "Mc",
            "N", "Nd", "Nl", "No", "P", "Pc", "Pd", "Ps", "Pe", "Pi", "Pf", "Po",
            "S", "Sm", "Sc", "Sk", "So", "Z", "Zs", "C", "Cc", "Cf", "Cn",
            "Alpha", "Digit", "Alnum", "Punct", "Upper", "Lower", "ASCII",
            "IsLatin", "IsGreek", "IsCyrillic"};
        return props[rng.nextInt(props.length)];
    }

    static String randomQuantifier() {
        int r = rng.nextInt(12);
        if (r < 2) return "*";
        if (r < 4) return "+";
        if (r < 6) return "?";
        if (r < 7) return "{" + rng.nextInt(4) + "}";
        if (r < 8) return "{" + rng.nextInt(3) + "," + (rng.nextInt(3) + 2) + "}";
        if (r < 9) return "{" + rng.nextInt(3) + ",}";
        if (r < 10) return "*?";
        if (r < 11) return "+?";
        return "??";
    }

    static String randomCharClass() {
        StringBuilder sb = new StringBuilder("[");
        if (rng.nextInt(4) == 0) sb.append("^");
        int items = rng.nextInt(4) + 1;
        for (int i = 0; i < items; i++) {
            if (rng.nextInt(3) == 0) {
                char start = (char)('a' + rng.nextInt(20));
                char end = (char)(start + rng.nextInt(6) + 1);
                if (end > 'z') end = 'z';
                sb.append(start).append("-").append(end);
            } else if (rng.nextInt(4) == 0) {
                String[] preds = {"\\d", "\\w", "\\s", "\\D", "\\W", "\\S"};
                sb.append(preds[rng.nextInt(preds.length)]);
            } else {
                sb.append(randomChar());
            }
        }
        sb.append("]");
        return sb.toString();
    }

    static String randomPattern(int maxDepth) {
        if (maxDepth <= 0) return randomChar();
        int terms = rng.nextInt(4) + 1;
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < terms; i++) {
            sb.append(randomAtom());
            if (rng.nextInt(3) == 0) sb.append(randomQuantifier());
        }
        if (rng.nextInt(5) == 0) {
            sb.append("|");
            sb.append(randomPattern(maxDepth - 1));
        }
        return sb.toString();
    }

    static String randomInput(String pattern) {
        // Mix of: chars that might match, random chars, empty
        int r = rng.nextInt(5);
        if (r == 0) return "";
        StringBuilder sb = new StringBuilder();
        int len = rng.nextInt(20) + 1;
        String alphabet = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 \t.,!@#$%";
        for (int i = 0; i < len; i++) {
            sb.append(alphabet.charAt(rng.nextInt(alphabet.length())));
        }
        return sb.toString();
    }

    static String randomFlags() {
        StringBuilder sb = new StringBuilder();
        if (rng.nextInt(5) == 0) sb.append("i");
        if (rng.nextInt(8) == 0) sb.append("m");
        if (rng.nextInt(10) == 0) sb.append("s");
        return sb.toString();
    }

    static int parseFlags(String flags) {
        int f = 0;
        if (flags.contains("i")) f |= Pattern.CASE_INSENSITIVE;
        if (flags.contains("m")) f |= Pattern.MULTILINE;
        if (flags.contains("s")) f |= Pattern.DOTALL;
        if (flags.contains("x")) f |= Pattern.COMMENTS;
        if (flags.contains("u")) f |= Pattern.UNICODE_CASE;
        if (flags.contains("d")) f |= Pattern.UNIX_LINES;
        return f;
    }

    static void emitTest(String pattern, String input, String flags) {
        String pid = String.format("D%05d", id++);
        try {
            int f = parseFlags(flags);
            // matches
            boolean matchResult = Pattern.compile(pattern, f).matcher(input).matches();
            out.println("{\"id\":\"" + pid + "m\",\"pattern\":\"" + esc(pattern) +
                "\",\"input\":\"" + esc(input) + "\",\"op\":\"matches\",\"expect\":" + matchResult +
                ",\"flags\":\"" + flags + "\"}");

            // find
            pid = String.format("D%05d", id++);
            Matcher m = Pattern.compile(pattern, f).matcher(input);
            StringBuilder sb = new StringBuilder("[");
            boolean first = true;
            int limit = 50;
            while (m.find() && limit-- > 0) {
                if (!first) sb.append(",");
                first = false;
                sb.append("{\"m\":\"").append(esc(m.group())).append("\"");
                if (m.groupCount() > 0) {
                    sb.append(",\"g\":[");
                    for (int i = 1; i <= m.groupCount(); i++) {
                        if (i > 1) sb.append(",");
                        if (m.group(i) == null) sb.append("null");
                        else sb.append("\"").append(esc(m.group(i))).append("\"");
                    }
                    sb.append("]");
                }
                sb.append("}");
            }
            sb.append("]");
            out.println("{\"id\":\"" + pid + "f\",\"pattern\":\"" + esc(pattern) +
                "\",\"input\":\"" + esc(input) + "\",\"op\":\"find\",\"expect\":" + sb +
                ",\"flags\":\"" + flags + "\"}");

            // replaceAll (sometimes)
            if (rng.nextInt(3) == 0) {
                pid = String.format("D%05d", id++);
                try {
                    String replacement = rng.nextInt(2) == 0 ? "X" : "[$0]";
                    String result = Pattern.compile(pattern, f).matcher(input).replaceAll(replacement);
                    out.println("{\"id\":\"" + pid + "r\",\"pattern\":\"" + esc(pattern) +
                        "\",\"input\":\"" + esc(input) + "\",\"op\":\"replaceAll\",\"expect\":\"" +
                        esc(result) + "\",\"replacement\":\"" + esc(replacement) +
                        "\",\"flags\":\"" + flags + "\"}");
                } catch (Exception e) { }
            }

            generated++;
        } catch (PatternSyntaxException e) {
            out.println("{\"id\":\"" + pid + "\",\"pattern\":\"" + esc(pattern) +
                "\",\"op\":\"compile_error\",\"expect\":true,\"flags\":\"" + flags + "\"}");
            generated++;
        } catch (StackOverflowError e) {
            skipped++;
        } catch (Exception e) {
            skipped++;
        }
    }

    public static void main(String[] args) throws Exception {
        long seed = args.length > 0 ? Long.parseLong(args[0]) : System.currentTimeMillis();
        int count = args.length > 1 ? Integer.parseInt(args[1]) : 2000;
        String outFile = args.length > 2 ? args[2] : "tests/java_regex_tests_diff.jsonl";

        rng = new Random(seed);
        out = new PrintWriter(new FileWriter(outFile));

        System.out.println("Seed: " + seed + ", generating " + count + " test patterns...");

        for (int i = 0; i < count; i++) {
            String pattern = randomPattern(rng.nextInt(3) + 1);
            String input = randomInput(pattern);
            String flags = randomFlags();
            emitTest(pattern, input, flags);
        }

        out.flush();
        out.close();
        System.out.println("Generated " + generated + " test patterns (" + id + " JSONL lines), skipped " + skipped);
    }
}
