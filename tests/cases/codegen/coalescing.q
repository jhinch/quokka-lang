global A: u8;
global RESULT: u8;

fn f() {
    let a = A;
    let b = a;
    let c = b;
    RESULT = c;
}

fn main() {
    f();
}
