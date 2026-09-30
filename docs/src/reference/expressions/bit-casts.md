# Bit cast expressions

```ebnf
BitCastExpression   =   ExpressionWithoutBlock  , "as" , TypeUse
                    ;
```

A bit cast expression transmutes the value on the left hand side to the type on the right hand side, producing a
value that is bit-identical.

Which types a value can be bit cast to are governed by the following rules:

- Both the source and destination types must have the same bit size
- The destination type must have a bit size greater than 0
- If the source and destination types are different, the both must have no padding
  including [explicit padding](../items/structs.md#paddingaignment)
- The type an integer literal on the left hand side will be inferred as the type of the right hand side

A value can always be bit cast to the type it is already.

Below are all examples of valid bit casts:

```quokka
struct BytePair {
    low: u8,
    high: u8,
}

fn main() {
    let a: u16 = 512;
    let b = a as u16;
    let c = b as i16;
    let d = c as BytePair;
    let e = d as [u8; 2];
}
```