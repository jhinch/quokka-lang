# Struct expressions

```ebnf
StructExpression    = Identifier , "{" , [ StructExprFields ] , "}"
                    ;
StructExprFields    =   StructExprField , { "," , StructExprField } , [ "," ]
                    ;
StructExprField     =   Identifier, ":", ExpressionWithoutBlock
                    ;
```