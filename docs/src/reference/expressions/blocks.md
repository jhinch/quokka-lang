# Block expressions

```ebnf
BlockExpression     =   "{" [ Statements ] "}"                          ;
Statements          =   Statement { Statement } ExpressionWithoutBlock
                    |   ExpressionWithoutBlock
                    ;
```