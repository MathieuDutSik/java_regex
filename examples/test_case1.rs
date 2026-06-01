use java_regex::Regex;
fn main() {
    let re = Regex::new(r"(?:((\1[^\w])*?)){2,3}?").unwrap();
    let result = re.matches("\t");
    println!("matches: {}", result);
}
