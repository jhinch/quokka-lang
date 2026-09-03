# Array index expressions

```ebnf
ArrayIndexExpression    =   Expression , "[" , Expression , "]"
                        ;
```

An array index expression evaluates the operand and the index expression and
produces the element of the [array](../types.md#array-types) at that index. The
index expression must be a `u16`.

## `local-array` preview feature

> [!WARNING]
> Requires `local-array` preview feature enabled.

Indexing a [local variable](../statements/lets.md) at runtime requires the
`local-array` preview feature to be enabled:

```quokka
global A: [u8; 2];

fn main() {
    let a = A;
    let first: u8 = a[0]; // requires `local-array`
}
```

The preview feature is _not_ required when the indexed operand is a
[global variable](../items/global-items.md) or when it appears inside a
[const item](../items/const-items.md):

```quokka
global A: [u8; 2];

const SECOND: u8 = {
    let a: [u8; 2] = [1, 2];
    yield a[1]; // ok, evaluated at compile time
};

fn main() {
    let first: u8 = A[0]; // ok, index on a global
}
```
