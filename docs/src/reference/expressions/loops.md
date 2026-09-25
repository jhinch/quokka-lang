# Loop expressions

```ebnf
LoopExpression      =     "loop" , BlockExpression
                    ;
```

## `const-loop` preview feature

> [!WARNING]
> Requires `const-loop` preview feature enabled.

Using a `loop` expression inside a [const item](../items/const-items.md)'s
initializer requires the `const-loop` preview feature to be enabled:

```quokka,preview=const-loop
const SUM: u8 = {
    let i = 0;
    let sum = 0;
    loop {
        if i == 5 then {
            break;
        }
        sum = sum + i;
        i = i + 1;
    }
    yield sum; // requires `const-loop`
};
```