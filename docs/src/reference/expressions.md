# Expressions

```ebnf
Expression              = ExpressionWithoutBlock
                        | ExpressionWithBlock
                        ;
ExpressionWithBlock     = BlockExpression
                        | IfExpressionWithBlock
                        | AsmBlockExpression
                        | LoopExpression
                        ;
ExpressionWithoutBlock  = GroupedExpression
                        | IfExpressionWithoutBlock
                        | LiteralExpression
                        | OperatorExpression
                        | ArrayIndexExpression
                        | PathExpression
                        | FieldAccessExpression
                        | FunctionCallExpression
                        ;
```

An expression is an element of the program which can be evaluated to produce a value.

An expression can have effects ("side effects") during evaluation.

The meaning of each kind of expression dictates:

- whether the expression contains sub-expressions (operands)
- under what circumstances operands are evaluated (including re-evaluation)
- how the values produced by operands combine to obtain the expressions value

## Expression precedence

The order in which expressions are evaluated is governed by a set of conventions.

- expression types can be grouped and ranked into levels
- this rank is called the expression precedence
- levels are ranked from _strongest_ (highest) to _weakest_ (lowest) precedence, where stronger precedence expressions
  are evaluated before weaker precedence
- [binary operator expressions](expressions/operators.md) within the same level are further ordered by the associativity,
  either from _left to right_, from _right to left_, or neither in which case they _require parentheses_
- expression precedence can be overridden using parentheses forming a [grouped expression](expressions/grouped.md)

| Expression type                                                                                                 | Associativity        |
|-----------------------------------------------------------------------------------------------------------------|----------------------|
| [Grouped](expressions/grouped.md)                                                                               | N/A                  |
| [Paths](expressions/paths.md)                                                                                   | N/A                  |
| [Field access](expressions/field-access.md)                                                                     | left to right        |
| [Function calls](expressions/function-calls.md), [Arrary index](expressions/array-index.md)                     | N/A                  |
| [Unary `-`](expressions/operators.md#negate-operator), [`!`](expressions/operators.md#not-operator)             | N/A                  |
| [Binary `-`, `+`](expressions/operators.md#arithmetic-operators)                                                | left to right        |
| [`<<`, `>>`](expressions/operators.md#shift-operators), [`<%`, `>%`](expressions/operators.md#rotate-operators) | left to right        |
| [`&`](expressions/operators.md#logical-operators)                                                               | left to right        |
| [`^`](expressions/operators.md#logical-operators)                                                               | left to right        |
| [`\|`](expressions/operators.md#logical-operators)                                                              | left to right        |
| [`==`, `!=`, `<`, `>`, `<=`, `>=`](expressions/operators.md#comparison-operators)                               | requires parentheses |
| [`&&`](expressions/operators.md#boolean-operators)                                                              | left to right        |
| [`\|\|`](expressions/operators.md#boolean-operators)                                                            | left to right        |
| [`=`, `+=`, `-=`, `&=`, `\|=`, `^=`, `<<=`, `>>=`, `<%=`, `>%=`](expressions/operators.md#assign-operators)     | right to left        |
