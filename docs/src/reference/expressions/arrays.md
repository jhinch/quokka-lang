# Array expressions

```ebnf
ArrayExpression     = "[" , ArrayExprItems , "]"
                    ;
ArrayExprItems      = Expression , { ",",  Expression } , [ "," ]
                    | Expression , ";" , NumberLiteral
                    ;
```

## Bulk fill

Assigning a repeat expression (`[value; N]`) directly to a
[global variable](../items/global-items.md) whose array type is larger than
16 bits lowers to a runtime fill loop instead of materializing the array in a local:

```quokka
global OAM: [u8; 160];

fn main() {
    OAM = [0; 160]; // runtime fill loop
}
```

This is restricted to:

- array item types `u8`/`i8` (other item types are not yet supported)
- a `value` that is a compile-time-constant literal integer (optionally
  negated for `i8`) - a runtime-computed value, or a reference to a
  [const item](../items/const-items.md), is not yet supported

See bulk copy in [operator expressions](operators.md)
for copying an oversized array/struct from another global or const.