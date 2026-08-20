global N: u16;

fn collatz_steps() -> u16 {
    let n = N;
    let steps = 0;
    loop {
        if n == 1 then {
            break;
        }
        let is_even = (n & 1) == 0;
        if is_even then {
            n = n >>;
        } else {
            n = n + n + n + 1;
        }
        steps += 1;
    }
    return steps;
}

global RESULT: u16;

fn main() {
    N = 27;
    RESULT = collatz_steps();
}
