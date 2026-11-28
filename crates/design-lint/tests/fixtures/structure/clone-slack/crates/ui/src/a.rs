pub fn first(x: &mut Vec<u32>, y: u32) -> u32 {
    let mut total = 0;
    for i in 0..x.len() {
        if x[i] > y {
            total += x[i] * 2;
            x[i] = 0;
        } else {
            total += 1;
        }
    }
    if total > 100 {
        return total - 100;
    }
    total
}
