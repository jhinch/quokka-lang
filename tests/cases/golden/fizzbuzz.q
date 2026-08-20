global MOD_x: u8;
global MOD_y: u8;

fn x_mod_y() -> u8 {
    let result = MOD_x;
    loop {
        if result < MOD_y then {
            break;
        }
        result -= MOD_y;
    }
    return result;
}

global N: u8;

fn fizzbuzz() -> u8 {
    MOD_x = N;
    MOD_y = 3;
    let mod3 = x_mod_y();
    MOD_y = 5;
    let mod5 = x_mod_y();
    if mod3 == 0 then {
        if mod5 == 0 then {
            return 3;
        }
        return 1;
    }
    if mod5 == 0 then {
        return 2;
    }
    return 0;
}

global RESULTS: [u8; 15];

fn main() {
    let i = 0;
    let n = 1;
    loop {
        if i == 15 then {
            break;
        }
        N = n;
        RESULTS[i] = fizzbuzz();
        i += 1;
        n += 1;
    }
}
