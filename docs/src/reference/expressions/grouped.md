# Grouped expressions

```ebnf
GroupedExpression   =   "(" , Expression , ")"
                    ;
```

A grouped expression can contain an [expression with a block](../expressions.md#expressions-with-blocks), which
allows it to be used as an operand:

```quokka
global G: u8;

fn main() {
    let x: u8 = ({
        yield G;
    }) + 1;
}
```