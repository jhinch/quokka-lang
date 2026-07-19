# If expressions

> [!WARNING]
> Requires `if-else` preview feature enabled.

```ebnf
IfExpressionWithoutBlock    =   "if" , Expression , "then" , ExpressionWithoutBlock , [ "else" , ExpressionWithoutBlock ]
                            ;
IfExpressionWithBlock       =   "if" , Expression , "then" , ExpressionWithBlock , [ "else" , ExpressionWithBlock ]
                            ;
```