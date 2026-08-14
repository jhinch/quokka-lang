# Struct expressions

```ebnf
StructExpression    = Identifier , "{" , [ StructField ] , "}"
                    ;
StructExprFields    =   StructExprField , { "," , StructExprField } , [ "," ]
                    ;
StructExprField     =   Identifier, ":", Expression
                    ;
```