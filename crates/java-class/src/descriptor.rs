// Descriptor display helper retained from the source DEX reader.
pub fn java_name(desc: &str) -> String {
    let dims = desc.chars().take_while(|&c| c == '[').count();
    let base = desc.get(dims..).unwrap_or("");
    let name = match base {
        "V" => "void".to_string(),
        "Z" => "boolean".to_string(),
        "B" => "byte".to_string(),
        "S" => "short".to_string(),
        "C" => "char".to_string(),
        "I" => "int".to_string(),
        "J" => "long".to_string(),
        "F" => "float".to_string(),
        "D" => "double".to_string(),
        b if b.starts_with('L') && b.ends_with(';') => b.get(1..b.len() - 1).unwrap_or("").replace('/', "."),
        b => b.to_string(),
    };
    format!("{name}{}", "[]".repeat(dims.min(255)))
}
