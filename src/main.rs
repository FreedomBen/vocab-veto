use banned_words_service::matcher::{LIST_VERSION, TERMS};

fn main() {
    println!("LIST_VERSION = {LIST_VERSION}");
    let mut codes: Vec<&&str> = TERMS.keys().collect();
    codes.sort();
    let mut total = 0usize;
    for code in codes {
        let terms = TERMS.get(*code).expect("key from TERMS.keys() missing");
        println!("  {:<4} {:>6} terms", code, terms.len());
        total += terms.len();
    }
    println!("total: {total} terms across {} languages", TERMS.len());
}
