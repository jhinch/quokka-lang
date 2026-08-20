global A: u16;
global B: u16;

fn gcd() -> u16 {
    let a = A;
    let b = B;
    loop {
        if b == 0 then {
            return a;
        }
        if b < a then {
            a -= b;
        } else {
            b -= a;
        }
    }
}

global RESULT: u16;

fn main() {
    A = 1071;
    B = 462;
    RESULT = gcd();
}
