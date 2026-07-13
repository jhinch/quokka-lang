# Statements

```ebnf
Statement           =   LetStatement
                    |   ReturnStatement
                    |   YieldStatement
                    |   BreakStatement
                    |   ContinueStatement
                    |   ExpressionStatement
                    ;
LetStatement        =   "let" Identifier [ ":" TypeUse ] "=" Expression ";"     ;
ReturnStatement     =   "return" [ Expression ] ";"                             ;
YieldStatement      =   "yield" Expression ";"                                  ;
BreakStatement      =   "break" [ Expression ] ";"                              ;
ContinueStatement   =   "continue" ";"                                          ;
ExpressionStatement =   ExpressionWithoutBlock ";"
                    |   ExpressionWithBlock
                    ;
```