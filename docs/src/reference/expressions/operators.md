# Operator expressions

```ebnf
OperatorExpression      =   NotExpression
                        |   NegateExpression
                        |   BooleanExpression
                        |   LogicalExpression
                        |   ArithmeticExpression
                        |   ComparisonExpression
                        |   ShiftExpression
                        |   RotateExpression
                        |   AssignExpression
                        ;
NotExpression           =   "!" , Expression
                        ;
NegateExpression        =   "-" , Expression
                        ;
LogicalExpression       =   Expression , "&&" , Expression
                        |   Expression , "||" , Expression
                        ;
ArithmeticExpression    =   Expression , "&" , Expression
                        |   Expression , "|" , Expression
                        |   Expression , "^" , Expression
                        |   Expression , "+" , Expression
                        |   Expression , "-" , Expression
                        ;
ComparisonExpression    =   Expression , "==" , Expression
                        |   Expression , "!=" , Expression
                        |   Expression , "<" , Expression
                        |   Expression , "<=" , Expression
                        |   Expression , ">=" , Expression
                        |   Expression , ">" , Expression
                        ;
ShiftExpression         =   Expression , "<<"
                        |   Expression , ">>"
                        ;
RotateExpression        =   Expression , "<%"
                        |   Expression , ">%"
                        ;
AssignExpression        =   PlaceExpression , "=" , Expression
                        |   PlaceExpression , "+=" , Expression
                        |   PlaceExpression , "-=" , Expression
                        |   PlaceExpression , "&=" , Expression
                        |   PlaceExpression , "|=" , Expression
                        |   PlaceExpression , "^=" , Expression
                        ;
```

## Not operator

- `!` is logical NOT for booleans and bitwise NOT for integers
- results in same type as input

## Negate operator

- `-` negates
- results in integer

## Boolean operators

- Operands must be booleans
- `&&` evaluates right operand if left is true
- `||` evaluates right operand if left is false
- Results in boolean

## Logical operators

- `&` is bitwise AND
- `|` is bitwise OR
- `^` is bitwise XOR
- Results in integer

## Arithmetic operators

- `+` adds
- `-` subs
- Results in integer

## Comparison operators

- `==` equal
- `!=` not equal
- `<` less than
- `<=` less than or equal
- `>=` greater than or equal
- `>` greater than
- operands are integers
- results in a boolean

## Shift operators

- `<<` shift left
- `>>` shift right (logical shift right if unsigned, arithmetic shift right if signed)

## Rotate operators

- `<%` rotate left
- `>%` rotate right

## Assign operators

- Operates the same as respective operator but assigns result left operand
- left operand must be a place expression