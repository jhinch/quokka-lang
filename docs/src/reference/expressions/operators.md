# Operator expressions

```ebnf
OperatorExpression      =   NotExpression
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
BooleanExpression       =   Expression , "&&" , Expression
                        |   Expression , "||" , Expression
                        ;
LogicalExpression       =   Expression , "&" , Expression
                        |   Expression , "|" , Expression
                        |   Expression , "^" , Expression
                        ;
ArithmeticExpression    =   Expression , "+" , Expression
                        |   Expression , "-" , Expression
                        ;
ComparisonExpression    =   Expression , "==" , Expression
                        |   Expression , "!=" , Expression
                        |   Expression , "<" , Expression
                        |   Expression , "<=" , Expression
                        |   Expression , ">=" , Expression
                        |   Expression , ">" , Expression
                        ;
ShiftExpression         =   Expression , "<<" , ShiftRhsOperand
                        |   Expression , ">>" , ShiftRhsOperand
                        ;
RotateExpression        =   Expression , "<%" , RotateRhsOperand
                        |   Expression , ">%" , RotateRhsOperand
                        ;
AssignExpression        =   Expression , "=" , Expression
                        |   Expression , "+=" , Expression
                        |   Expression , "-=" , Expression
                        |   Expression , "&=" , Expression
                        |   Expression , "|=" , Expression
                        |   Expression , "^=" , Expression
                        |   Expression , "<<=" , ShiftRhsOperand
                        |   Expression , ">>=" , ShiftRhsOperand
                        |   Expression , "<%=" , RotateRhsOperand
                        |   Expression , ">%=" , RotateRhsOperand
                        ;
ShiftRhsOperand         =   "1"
                        ;
RotateRhsOperand        =   "_"
                        |   Expression
                        ;
```

## Not operator

- `!` is logical NOT for booleans and bitwise NOT for integers
- results in same type as input

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
- `>>` shift right (logical if unsigned, arithmetic if signed)
- right operand must always be literal `1`

## Rotate operators

- `<%` rotate left
- `>%` rotate right
- when right operand is `_`, rotates in place
- when right operand is expression, must be a boolean

## Assign operators

- Operates the same as respective operator but assigns result left operand