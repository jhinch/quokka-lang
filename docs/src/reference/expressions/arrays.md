# Array expressions

```ebnf
ArrayExpression     = "[" , ArrayExprItems , "]"
ArrayExprItems      = Expression , { "," Expression } , [ "," ]
                    | Expression , ";" , NumberLiteral
                    ;
```