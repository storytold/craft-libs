pub fn second(v: &mut Vec<u32>, y: u32) -> u32 {
    let mut sum = 0;
    for i in 0..v.len() {
        if v[i] > y {
            sum += v[i] * 2;
            v[i] = 0;
        } else {
            sum += 1;
        }
    }
    if sum > 100 {
        return sum - 100;
    }
    sum
}
