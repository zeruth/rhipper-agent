# The tally ledger format

This document is the specification of the ledger format read by `tally` and written by hand. It is
long because a specification is meant to be complete: every rule has its own section, and every
section ends with examples. A reader of one section does not need the others, except where a
section says it does.

## 1. Purpose

A ledger is a plain text file in which a person writes down what was spent, one thing to a line,
and a program adds it up. The format exists so that the writing is quick, the file is readable
years later without the program, and the program has very little to decide.

Three aims explain most of the rules that follow.

The first aim is that a ledger can be written without looking anything up. A line is a category and
an amount, in that order, and nothing else is required. There is no date, no currency symbol, no
account, no tag. A person who wants those writes them in a comment.

The second aim is that a mistake is found where it was made. A reader of the format does not guess.
A line that is not an entry is an error that names the line, and the report is not printed at all
when any line is bad. A report that sums a ledger with a mistyped line in silence is worse than no
report, because it is believed.

The third aim is that the format stays small. Every feature a ledger could use has a cost in the
reader and in the person who must remember it, and the format has been kept to what a month of
spending needs. Where a feature was asked for and refused, the history in section 12 says so.

The words must, must not and may are used in their ordinary sense. A reader of the format must do
what a section says it must. A reader may do what a section says it may, and a ledger must not
depend on it.

### 1.1 The files

A ledger file is text in UTF-8. By custom it ends in `.ledger`, but the name is not part of the
format and a reader must not look at it. A reader takes the file as a sequence of lines, split at
the line feed. A carriage return before a line feed is whitespace at the end of the line and is
handled by section 5 like any other.

## 2. Lines

A ledger is a sequence of lines. Every line is one of three things: a comment, a blank line, or an
entry. Comments and blank lines are the subject of section 6. This section is about entries.

An entry is two words. The first word is the category of the spending and the second is the amount.
The words are separated by whitespace. Nothing may come before the first word except whitespace,
and nothing may follow the second except whitespace.

```
food 12.50
rent 800
transport 3.2
```

A line has no other structure. There is no separator but whitespace, no quoting, and no escape: a
category that would need a space in it is written with a dash instead (section 3), and an amount is
written as digits.

### 2.1 Words

A word is a run of characters that are not whitespace. The first word of an entry is read as a
category and the second as an amount, and each is held to the rules of its own section. Whether
a word is acceptable is a question for section 3 or section 4, not for this one: this section only
says how many words there are.

A line with one word is not an entry: it has a category and no amount, and it is an error (section
7). A line with three or more words is not an entry either, because the format has no third field.
Before this was decided a reader dropped the extra words and read the first two, so that
`food 12.50 lunch` was read as `food 12.50`. That is the fault the rule exists to prevent: a note
written at the end of a line is lost without warning, and a word that was meant as part of the
amount, as in `food 12 50`, is dropped and the sum is wrong. A note belongs on a comment line
above the entry.

### 2.2 Lines are independent

An entry means the same wherever it stands. No line refers to another, and no line changes how the
lines after it are read. There are no directives, no includes and no variables.

### 2.3 Examples

These are entries:

```
food 12.50
food 12.5
food 12
rent 800.00
refund-food -3.20
```

These are not entries, and each is an error:

```
food
food 12.50 lunch
food twelve
Food 12.50
12.50 food
```

The first has no amount. The second has a third word. The third has an amount that is not one. The
fourth has a category that is not a name, because of its capital letter. The fifth has its words
the wrong way round: the first word, `12.50`, is not a category name, and that is the error that
is reported (section 7).

## 3. Categories

A category is the name under which an amount is summed. Two entries with the same category are
added together in the report, and two with categories that differ at all are not.

A category name is made of lowercase letters from a to z, digits from 0 to 9, and the dash. It is
at least one character long. Nothing else is allowed in it: not a capital letter, not an
underscore, not a space, not a letter with an accent, not a dot, not a slash.

```
food
rent
eating-out
q3-travel
7-eleven
```

These are not names:

```
Food
eating_out
cafe-au-lait-é
food.lunch
```

### 3.1 Why so narrow

The set is narrow so that the same thing cannot be written in two ways. If capital letters were
allowed, `Food` and `food` would both be written on different days, and the report would show two
lines for one kind of spending. The dash is allowed so that a name of more than one word can be
written without a space, which would split it into two words (section 2).

A name may begin with a digit or a dash: `7-eleven` and `-5` are names. The two words of an entry
are told apart by position and not by their look: the first is always the category.

### 3.2 The error for a bad name

A category that has any character outside the set is an error. The error carries the name as
written, so that the person finds it in the line without counting. Section 7 has the sentence.

When a line has both a bad category and a bad amount, the category is the one that is reported. The
words are read in order, and the first fault found is the fault reported.

### 3.3 Examples

| Written | Category | Result |
|---|---|---|
| `food 12.50` | `food` | an entry |
| `eating-out 7` | `eating-out` | an entry |
| `7-eleven 2.40` | `7-eleven` | an entry |
| `Food 12.50` | `Food` | error: not a category name |
| `food_court 5` | `food_court` | error: not a category name |
| `12.50 food` | `12.50` | error: not a category name |

## 4. Amounts

An amount is a number of units and, optionally, a number of hundredths of a unit. The format does
not say what the unit is. A ledger is in one currency by the person's own arrangement, and nothing
in a line says which.

An amount is written as whole units: one or more digits. After them there may be a dot and one or
two digits, the decimals. There is no thousands separator, no exponent, no currency symbol and no
percent sign.

```
12
12.5
12.50
0.5
0.05
800
```

`12.5` is twelve units and fifty hundredths, the same as `12.50`. A single decimal digit is tens of
hundredths, not hundredths: `0.5` is fifty hundredths and not five.

A reader holds amounts as a whole number of hundredths, which this document calls cents whatever
the unit. A reader must not use fractions of any kind, so that a sum of any number of lines is
exact.

### 4.1 What is not an amount

A word is not an amount in any of these cases.

- It is empty after the dot: `12.` is not an amount.
- It has more than two decimals: `1.234` is not an amount. The format does not round.
- It has a character that is not a digit where a digit is needed: `12,50`, `1e3`, `$12` and
  `12.5x` are not amounts. A comma is not a decimal point.
- It has no units: `.50` is not an amount, and is written `0.50`.
- It is a word: `twelve` is not an amount.

A word that is not an amount is an error, and the error carries the word as written (section 7).

### 4.2 Negative amounts

An amount may be negative. A negative amount is a minus sign, `-`, directly followed by an amount
as described above, with nothing between them. It means the negative of the amount that follows
the sign. It is how a refund, a correction or a return is written: the original line is left alone
and a new line with the opposite amount follows it.

```
food 12.50
food -12.50
refund-rent -400
```

`-1.50` is minus one unit and fifty hundredths, which is minus a hundred and fifty cents. It is
not minus one unit plus fifty hundredths. This is the point of the rule that is easiest to get
wrong, and the example that follows should be kept in mind: `-0.50` is minus fifty cents, and a
reader that takes the units, `-0`, as zero and adds the fifty hundredths reads it as plus fifty.

Only one sign is allowed. `--1.50` is not an amount. A sign must be directly before the digits:
`- 1.50` is two words and the line has three. A minus sign is not allowed anywhere else in an
amount, so that `1-50` and `1.-50` are not amounts.

The sign of a negative zero is not preserved: `-0` and `-0.00` are amounts, and they are zero.

A reader prints a negative number of cents as one minus sign, then the units, then the two
decimals. The negative of 150 cents is `-1.50`, and of 50 cents is `-0.50`. It never prints the
minus sign twice, and it never prints it between the units and the decimals.

### 4.3 The plus sign

The format has no plus sign. A positive amount is written without a sign. What a reader does with
a word that begins with `+` is not defined by this document, and a ledger must not contain one.

### 4.4 Examples

| Written | Cents | Result |
|---|---|---|
| `12` | 1200 | an amount |
| `12.5` | 1250 | an amount |
| `12.50` | 1250 | an amount |
| `0.05` | 5 | an amount |
| `-1.50` | -150 | an amount |
| `-0.50` | -50 | an amount |
| `--1.50` | | error: not an amount |
| `1.234` | | error: not an amount |
| `12.` | | error: not an amount |
| `.5` | | error: not an amount |
| `12,50` | | error: not an amount |

## 5. Whitespace

Whitespace separates the two words of an entry and may surround them. It carries no other meaning.

A space and a tab are whitespace. So is a carriage return, which is why a ledger written on a
system that ends its lines with a carriage return and a line feed is read as the same ledger as one
written on a system that ends them with a line feed alone. A reader must treat a trailing carriage
return as whitespace and must not fail on it.

Any amount of whitespace may stand between the two words, and any amount before the first and after
the second. These lines are the same entry:

```
food 12.50
food    12.50
   food 12.50
food 12.50   
	food	12.50
```

Whitespace is not part of a word. A category with a space in it is two words, and an amount with a
space in it is two words, which is a line of three (section 2).

### 5.1 Examples

| Written | Words | Result |
|---|---|---|
| `food 12.50` | `food`, `12.50` | an entry |
| `  food   12.50  ` | `food`, `12.50` | the same entry |
| `food 12.50\r` | `food`, `12.50` | the same entry |
| `food 12 50` | `food`, `12`, `50` | error: more than two words |
| `eating out 5` | `eating`, `out`, `5` | error: more than two words |

## 6. Comments and blank lines

A line whose first character that is not whitespace is `#` is a comment. A line that is empty or only whitespace is blank. A reader skips both. A `#` anywhere else is not a comment: `food 1#2` is an entry with a malformed amount. A skipped line still has a number: the line number in an error counts every line of the file, skipped or not.

## 7. Errors

A reader reports the first line that is not an entry, a comment or a blank line, and reads no
further. It reports the number of the line, counting from one and counting every line of the file
(section 6), and the sentence for the fault. A reader that reports more than one fault must still
report the first one first. When any line is bad, the report is not printed.

There are five faults and five sentences. The sentences are exact: a program that reads the
command line's standard error may compare with them.

| Fault | The sentence |
|---|---|
| A line that has no words at all, where an entry was required | `empty line` |
| An entry with a category and no amount | `no amount after the category` |
| An amount that is not one | `not an amount: ` followed by the word as written |
| A line with more than two words | `more than two words` |
| A category that is not a name | `not a category name: ` followed by the name as written |

### 7.1 When each applies

`empty line` is the fault of a function that is given a line to read as an entry and is given
nothing. A ledger file does not raise it, because a blank line in a file is skipped by section 6
before it is read as an entry. It is the fault of a reader that is asked for the entry on a line
that has none.

`no amount after the category` is the fault of a line of one word.

`not an amount: ` is the fault of a second word that is not an amount by section 4. The word is
shown as written, whatever it was: `not an amount: 1.234`, `not an amount: twelve`,
`not an amount: --1.50`.

`more than two words` is the fault of a line of three words or more. It is raised before the words
are looked at, so that `food 12 50` is reported as having more than two words, and not as having an
amount of `12` and a word left over.

`not a category name: ` is the fault of a first word that has a character outside the set of
section 3. The name is shown as written: `not a category name: Food`, `not a category name: 12.50`.

### 7.2 Which comes first

When a line has more than one fault, the order in which they are checked is: whether it has any
words (`empty line`), whether it has more than two (`more than two words`), whether it has an
amount (`no amount after the category`), whether the category is a name (`not a category name: `),
and last whether the amount is one (`not an amount: `). The one that is reported is the first of
these that applies.

So `Food twelve` is reported as `not a category name: Food`, and `food twelve 5` is reported as
`more than two words`.

### 7.3 The line number

The sentence is preceded, when it is shown by the command line, by the number of the line and a
colon. The line number counts from one, and counts every line of the file, including the comments
and the blank lines that were skipped. A bad line on the fourth line of the file, after a comment
and two entries, is line 4. A bad line after two blank lines and an entry is line 4 as well.

```
line 4: not an amount: 1.234
```

The form is `line ` and the number, a colon, a space, and the sentence. A reader that shows the
errors as a person reads them uses this form.

### 7.4 Examples

| Written | Reported |
|---|---|
| `food` | `no amount after the category` |
| `food 12.50 lunch` | `more than two words` |
| `food 1.234` | `not an amount: 1.234` |
| `Food 12.50` | `not a category name: Food` |
| `12.50 food` | `not a category name: 12.50` |

## 8. The report

The report is what a reader prints for a ledger that has no bad line. It has one line for each
category, then a last line for the total.

The category lines are in the order of the category names, which is the order of their bytes:
digits before letters, and the dash before the digits. Each is the category, a space, and the sum
of its entries in the form of section 9.

The last line is the word `total`, a space, and the total.

```
food 15.00
rent 800.00
total 815.00
```

### 8.1 What the report leaves out

The report has no heading, no date, no count of lines and no percentages. A reader may offer these
as options, and what the report is without them is what this section says.

A ledger with no entries has a report of one line, the total, which is zero:

```
total 0.00
```

A ledger that has only comments and blank lines is such a ledger.

### 8.2 The form of a sum

A sum is printed as units, a dot, and two decimals. It is never printed in any other form, with
no thousands separator and no currency symbol. A sum of 1250 cents is `12.50`, a sum of 5 is
`0.05`, and a sum of 80000 is `800.00`.

A negative sum is printed with a minus sign directly before the units: `-1.50`. A sum that is
negative and less than a unit is `-0.50`, with the zero of the units shown.

### 8.3 Categories whose sum is zero

A category is in the report if it has any entry, whatever its sum is. A category whose entries sum
to zero is shown with `0.00`, because the person who wrote the entries wants to see that they
cancelled, and not to wonder where they went.

### 8.4 Example

For this ledger:

```
# January
food 12.50
rent 800

food 2.50
food -2.50
transport 3.2
```

the report is:

```
food 12.50
rent 800.00
transport 3.20
total 815.70
```

## 9. Totals

The total is the sum, in cents, of every entry of the ledger. It is the sum of the category sums,
and equal to it exactly, because every amount is a whole number of cents.

Negative amounts count as what they are. A ledger with `food 10` and `food -4` has a food sum of
six and a total of six.

The total may be negative, when refunds are larger than spending, and it is then printed as a
negative sum by section 8. It is not clamped at zero. Two readers of the same ledger print the
same total to the cent, because amounts are whole cents and never fractions.

A reader may offer a category's share of the total, in whole percent, rounded to the nearest whole
percent with halves up. A share is zero when the total is zero or less, when the category is not
in the ledger, and when its sum is negative, and it is never more than 100.

### 9.1 Examples

| Ledger | Total |
|---|---|
| `food 1` and `food 2.50` | `3.50` |
| `food 10` and `food -4` | `6.00` |
| `food 5` and `food -5` | `0.00` |
| `food 1` and `food -2.50` | `-1.50` |
| no entries | `0.00` |

## 10. Exit codes

The command line `tally-cli` reports its outcome by its exit code as well as by what it prints.

| Code | Meaning |
|---|---|
| 0 | The ledger was read and its report printed on standard output. |
| 1 | A line of the ledger is bad. The line and its fault are printed on standard error, and nothing is printed on standard output. |
| 2 | The ledger could not be read: no file was named, or the file could not be opened. The reason is printed on standard error. |

A script may rely on these three and on no others. The report goes to standard output only, and
every message that is not the report goes to standard error only, so that a pipe carries the
report and nothing else.

### 10.1 Reading the standard input

A command line may read its ledger from standard input when it is given a single dash as its file
name. It behaves as it does for a file: the report on standard output and exit 0, or the first bad
line on standard error and exit 1.

### 10.2 Comments and the exit code

A ledger that has only comments and blank lines has a report, the zero total, and exits 0. It is
not an error that a ledger is empty.

## 11. Worked examples

### 11.1 A month

```
# January, household
rent 800

food 12.50
food 7.25
transport 3.2

# a refund
food -4
```

Lines 1, 3 and 7 are a comment, a blank line and a comment. Line 2 is the entry `rent` with
80000 cents. The food entries are 1250, 725 and -400, which sum to 1575. Transport is 320.

```
food 15.75
rent 800.00
transport 3.20
total 819.95
```

### 11.2 A bad line after skipped ones

```
# January

food 12.50
food 1.234
rent 800
```

Line 1 is a comment and line 2 is blank, both skipped. Line 3 is an entry. Line 4 has an amount of
three decimals. The ledger is bad at line 4, and the reader prints this on standard error, exits
with code 1, and prints nothing on standard output:

```
line 4: not an amount: 1.234
```

The line is numbered 4 and not 2, because the line number counts every line of the file.

### 11.3 A line that is mistaken for a comment

`food 1#2` is not a comment, because `#` is not the first character of the line that is not
whitespace. It is an entry with the amount `1#2`, which is not an amount: `line 1: not an amount: 1#2`.

### 11.4 A refund smaller than a unit

The lines `food 3` and `food -0.50` are 300 and -50 cents, and the sum is 250: `food 2.50`. A reader
that took `-0.50` as minus zero plus fifty cents would print `3.50`, which section 4.2 rules out.

## 12. History of the format

The format has been written down in this form for as long as the program has existed, and the
rules were not all there at the start. This section says how it came to be what it is, so that a
person who wants to change a rule can see what was tried.

### 12.1 The first form

The first form was a category and an amount, separated by a space, and nothing else. A reader
split the line at whitespace and took the first two words. A third word was dropped.

That was a fault from the beginning, and it was found by a person whose ledger had `food 12 50`
for `food 12.50`, which was read as `food 12` for months. The rule of section 2.1, that a line has
two words and not more, is the cure. The cure was refused for a time on the ground that a note at
the end of a line was convenient, and it was accepted when it was seen that the note could as well
be a comment.

### 12.2 Comments

Comments came second. The first ledgers were machine-written and had no need of them, and then
people began to write ledgers by hand and wanted to say why. The rule is that `#` is a comment at
the start of a line, ignoring whitespace, and nowhere else. A comment at the end of an entry was
proposed and refused, because `#` could then be the start of an amount, and the line `food 1#2`
would be a different thing according to the reader.

Blank lines were allowed at the same time. A reader that does not skip a blank line is one that
stops the whole report for a line the person cannot see, and the first readers did exactly that.

The line numbers of errors counting the skipped lines was settled then, after a version that did
not count them: its errors named a line a few places before the real one, and the person had to
count comments by hand.

### 12.3 Names

Category names were free text at first. Capital letters came into ledgers on the second day, and
the report then showed `Food` and `food` as two lines. The set of section 3 was chosen to make
that impossible. Names with accents were asked for and refused for the same reason: the same word
can be written two ways in Unicode, and a person cannot see which they have written.

The underscore was in the first set and was taken out when the dash was found to be enough.

### 12.4 Amounts

Amounts were whole units at first. Decimals came with the second ledger. A third decimal was
allowed for a short while and was rounded by the reader, and the rounding was different in two
readers, which is why section 4 now refuses more than two and why a sum is held in whole cents.

Negative amounts came last, with refunds. The first form of the rule was that a leading minus
negated the units, so that `-1.50` was minus one plus fifty hundredths, which was wrong and was
found when a refund of less than a unit was written as `-0.50` and printed as plus fifty. Section
4.2 says what it is now.

### 12.5 What may change

The five sentences of section 7 are the part of the format that other programs read, and they
change last and least. A new fault gets a new sentence and the old ones stay as they are. The
form of the report is the next most fixed. The rest may be added to, and a ledger that was
correct under an earlier form is correct under a later one.
