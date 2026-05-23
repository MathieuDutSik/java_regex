fn main() {
    let re = java_regex::Regex::new(r"(?:I*(?:\s{0,3}){0})+?").unwrap();
    let matches = re.find("!F2GyZerbg!ldxa.4l");
    println!("Got {} matches", matches.len());
    for m in &matches {
        println!("  '{}' at {}-{}", m.matched_text, m.start, m.end);
    }
}
