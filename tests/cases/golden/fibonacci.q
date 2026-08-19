global FIBONACCI_N: u8;

fn fibonacci() -> u8 {
    let n = FIBONACCI_N;
    if n == 0 then {
        return 0;
    }
    n += 1;
    let a = 0;
    let b = 1;
    let i = 2;
    loop {
        if i >= n then {
            break;
        }
        let c = a + b;
        a = b;
        b = c;
        i += 1;
    }
    return b;
}

global RESULTS: [u8; 10];

fn main() {
    let i = 0;
    let n = 0;
    loop {
        if i == 10 then {
            break;
        }
        FIBONACCI_N = n;
        RESULTS[i] = fibonacci();
        i += 1;
        n += 1;
    }
}