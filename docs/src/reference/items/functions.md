# Functions

```ebnf
Function    =   "fn" , Identifier , "(" , ")" , [ "->" , TypeUse ] , BlockExpression
            ;
```

```quokka
fn this_function() {
}

fn that_function() -> u8 {
    return 1;
}
```

## Placing a function at a specific address

By default, a function is placed wherever the linker finds room for it. The `region` and `address`
attributes pin a function to a specific location in ROM instead, the same way they do for
[const items](const-items.md#using-specific-regions-of-memory).

```quokka
#[address = 0x0150]
fn checkpoint() {}
```

- `address` (u16) - The starting address of the function. Defaults to the compiler automatically choosing.
- `region` - Specifies the ROM region the function is placed in. Must be one of the following values:
  - `auto` (default) - The compiler automatically chooses `rom0` or `romx` based on `address`
  - `rom0` - Fixed (bank 0) ROM
  - `romx` - Banked ROM

A function's address must not overlap the cartridge header (`$0100`-`$014F`), which is always
reserved by the compiler; doing so is a compilation error.