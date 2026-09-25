# Field access expressions

```ebnf
FieldAccessExpression   =   Expression  , "." , Identifier
                        ;
```

A field access expression evaluates the operand and produces the value of the
named field.

## `local-field-access` preview feature

> [!WARNING]
> Requires `local-field-access` preview feature enabled.

Accessing a field of a [local variable](../statements/lets.md) at runtime
requires the `local-field-access` preview feature to be enabled:

```quokka,preview=local-field-access
struct Point { x: u8, y: u8 }
global P: Point;

fn main() {
    let p = P;
    let x: u8 = p.x; // requires `local-field-access`
}
```

The preview feature is _not_ required when the field access operand is a
[global variable](../items/global-items.md) or when it appears inside a
[const item](../items/const-items.md), as those forms are evaluated without
needing to materialise the aggregate in a local:

```quokka
struct Point { x: u8, y: u8 }
global P: Point;

const ORIGIN_X: u8 = {
    let p: Point = Point { x: 0, y: 0 };
    yield p.x; // ok, evaluated at compile time
};

fn main() {
    let x: u8 = P.x; // ok, field access on a global
}
```