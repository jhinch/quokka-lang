# Block expressions

```ebnf
BlockExpression     =   "{" , { Statement } , "}"
                    ;
```

Block expressions are a series of statements which result in a value.

Unlike other languages, the tail expression of a block does not impact the result of the expression. Instead,
[`yield` and `break`](../statements.md) statements impact the type. If the block contains no such statements,
the result is unit.

```quokka
fn main() {
    let x: u8 = {
        yield 1;
    };
}
```

A block expression is an [expression with a block](../expressions.md#expressions-with-blocks), so it cannot be used
directly as an operand. Wrap it in a [grouped expression](grouped.md) instead:

```quokka
global G: u8;

fn main() {
    let x: u8 = 0;
    x = {
        yield G;
    };
    x += ({
        yield 1;
    }) + 1;
}
```

- `yield` cannot be used in the top level block of a function (its body)
- `yield` cannot be used in a loop block. Use `break` instead
