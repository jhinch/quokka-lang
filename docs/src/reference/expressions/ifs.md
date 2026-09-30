# If expressions

```ebnf
IfExpressionWithoutBlock    =   "if" , ExpressionWithoutBlock , "then" , ExpressionWithoutBlock
                            |   "if" , ExpressionWithoutBlock , "then" , Expression , "else" , ExpressionWithoutBlock
                            ;
IfExpressionWithBlock       =   "if" , ExpressionWithoutBlock , "then" , ExpressionWithBlock
                            |   "if" , ExpressionWithoutBlock , "then" , Expression , "else" , ExpressionWithBlock
                            ;
```

Each branch can independently be an [expression with a block](../expressions.md#expressions-with-blocks) or
without one. The last branch determines which kind the whole `if` expression is: it is an expression with a block when
its last branch is one.

```quokka
global G: u8;

fn main() {
    let c = G;
    let x: u8 = if c == 1 then {
        yield 1;
    } else 2;
    if c == 2 then G = x else {
        G = 0;
    }
}
```