# If expressions

```ebnf
IfExpressionWithoutBlock    =   "if" Expression "then" ExpressionWithoutBlock [ "else" ExpressionWithoutBlock ]     ;
IfExpressionWithBlock       =   "if" Expression "then" ExpressionWithBlock [ "else" ExpressionWithBlock ]           ;
```