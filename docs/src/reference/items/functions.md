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