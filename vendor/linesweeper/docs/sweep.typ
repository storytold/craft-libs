#import "@preview/ctheorems:1.1.3": *
#import "@preview/cetz:0.4.0"
#import "@preview/lovelace:0.3.0": *
#set par(justify: true)
#set math.equation(numbering: "(1)")

#show: thmrules
#let lemma = thmbox("lemma", "Lemma")
#let def = thmbox("lemma", "Definition")
#let proof = thmproof("proof", "Proof")
#let remark = thmbox("lemma", "Remark")

#let inexact(term) = {
  block(inset: 16pt,
    block(
      inset: 16pt,
      stroke: 1pt + gray,
      radius: 12pt,
      text(
        size: 10pt,
        [Note for inexact arithmetic: #term]
      )
    )
  )
}

// TODO: figure out how to get rid of the ":" in the caption
#let invariant(term) = {
figure(
  block(inset: 16pt,
      text(
        size: 10pt,
        [#term]
    )
  ),
  kind: "invariant",
  supplement: "invariant",
  caption: ""
)
}

#let fl = text([fl])

We'll be talking about sweep line algorithms, where the sweep line is horizontal and increasing in $y$.
Therefore, every line segment "starts" at the coordinate with smaller $y$ and "ends" at the coordinate
with larger $y$ (we'll assume for now that there are no horizontal segments). We'll parametrize each
line segment as a function of $y$. So if $alpha: [y_0 (alpha), y_1 (alpha)] arrow bb(R)$ is a line segment
then $alpha(y)$ is the $x$ coordinate at $y$-coordinate $y in [y_0 (alpha), y_1 (alpha)]$.
We write $theta_alpha$ for the angle that $alpha$ makes with the positive horizontal axis.
Let's have a picture. (In the discussion, it won't matter whether positive $y$ points up or down, but in the
pictures we'll adopt the graphics convention of having positive $y$ point down.)

#figure(
cetz.canvas({
  import cetz.draw: *

  line((1, 3), (0, 0), name: "a")
  line((-4, 3), (4, 3), stroke: (dash: "dotted"))
  line((-4, 0), (4, 0), stroke: (dash: "dotted"))
  content((-4, 3), $y_0(alpha)$, anchor: "east")
  content((-4, 0), $y_1(alpha)$, anchor: "east")
  content((0.6, 1.5), $alpha$, anchor: "west")

  cetz.angle.angle("a.start", "a.end", (4, 3), label: $theta_alpha$, label-radius: 0.8)
}),

caption: "A segment and its angle"
)

We'll be dealing with inexact arithmetic, so let's define some "error bars" on our line segments.
For an error parameter $epsilon > 0$, offsetting from $alpha$ by $plus.minus epsilon$ in the perpendicular-to-$alpha$ direction
is the same as offsetting by $alpha plus.minus epsilon / (|sin theta_alpha|)$ in the horizontal direction.
Roughly speaking, the "error bars" on $alpha$ amount to adding this horizontal error. But we'll be slightly
more accurate around the corners, by truncating these error bars to the horizontal extents of $alpha$. Precisely, we define

$
alpha_(+,epsilon)(y) = min(alpha(y) + epsilon / (|sin theta_alpha|), max(alpha(y_0), alpha(y_1)) + epsilon) \
alpha_(-,epsilon)(y) = max(alpha(y) - epsilon / (|sin theta_alpha|), min(alpha(y_0), alpha(y_1)) - epsilon) \
$

In pictures, the gray shaded region is the region between $alpha_(-,epsilon)$ and $alpha_(+,epsilon)$:
The point of the scaling by $|sin theta_alpha|$ is to make this an approximation of an $epsilon$-neighborhood (in the
perpendicular direction) of the line segment. The truncation near the corners ensures that if $x$ is between
$alpha_(-,epsilon)(y)$ and $alpha_(+,epsilon)(y)$ then it is within $sqrt(2) epsilon$ of $alpha$.

#figure(
cetz.canvas({
  import cetz.draw: *

  line((0.5, 3), (-0.3, 0.6), (-0.3, 0), (0.5, 0), (1.3, 2.4), (1.3, 3), close: true, fill: gray, stroke: 0pt)
  line((0.5, 3), (-0.3, 0.6), (-0.3, 0), stroke: ( dash: "dashed" ))
  line((0.5, 0), (1.3, 2.4), (1.3, 3), stroke: ( dash: "dashed" ))

  line((1, 3), (0, 0), name: "a")
  line((-4, 3), (4, 3), stroke: (dash: "dotted"))
  line((-4, 0), (4, 0), stroke: (dash: "dotted"))
  content((-4, 3), $y_0(alpha)$, anchor: "east")
  content((-4, 0), $y_1(alpha)$, anchor: "east")
  content((0.6, 1.5), $alpha$, anchor: "west")

  content((0.8, 0.5), $alpha_(+,epsilon)$, anchor: "west")
  content((0.2, 2.4), $alpha_(-,epsilon)$, anchor: "east")
}),
caption: "A segment and its error bars"
)<fig-error-bars>


Define a relation $prec_(y,epsilon)$ on line segments whose domain contains $y$, where
$alpha prec_(y,epsilon) beta$ if $alpha_(+,epsilon)(y) < beta_(-,epsilon)(y)$.
Intuitively, $alpha prec_(y,epsilon) beta$ if $alpha$ is definitely to the left of $beta$
at height $y$: $alpha$ is far enough to the left that their error bars don't overlap.
Clearly, for a given $y$ and $epsilon$ there are three mutually exclusive possibilities: either
$alpha prec_(y,epsilon) beta$ or $beta prec_(y,epsilon) alpha$ or neither of the two holds. We'll denote
this third possibility by $alpha approx_(y,epsilon) beta$, and we write
$alpha prec.tilde_(y,epsilon) beta$ for "$alpha prec_(y,epsilon) beta$ or $alpha approx_(y,epsilon) beta$."

Here are a few basic properties of our definitions:
#lemma[
1. For any $y$ and any $epsilon > 0$, $prec_(y,epsilon)$ is transitive:
  if $alpha prec_(y,epsilon) beta$
  and $beta prec_(y,epsilon) gamma$ then $alpha prec_(y,epsilon) gamma$. (However, $prec.tilde_(y,epsilon)$ is not transitive.)
2. For any $y$ and any $epsilon > 0$,
  if $alpha prec_(y,epsilon) beta$
  and $beta prec.tilde_(y,epsilon) gamma$ then $alpha prec.tilde_(y,epsilon) gamma$.
3. For any $y$ and any $epsilon > 0$,
  if $alpha prec.tilde_(y,epsilon) beta$
  and $beta prec_(y,epsilon) gamma$ then $alpha prec.tilde_(y,epsilon) gamma$.
4. For any $y$, the relation $prec_(y,epsilon)$ is monotone in $epsilon$, in that if $alpha prec_(y,epsilon) beta$ then $alpha prec_(y,eta) beta$ for
  any $eta in (0, epsilon)$.
]<lem-basic-order-properties>

Since $epsilon$ for us will usually be fixed, we will often drop it from the notation, and write $alpha_-$ and $alpha_+$ instead
of $alpha_(-,epsilon)$ and $alpha_(+,epsilon)$.

#def[
  Suppose $alpha$ and $beta$ are two segments whose domain contains $y$. We say that *$(alpha, beta)$
  $epsilon$-cross by $y$* if $y$ belongs to both domains and $alpha succ_(y,epsilon) beta$.
  We say that *$(alpha, beta)$ $epsilon$-cross* if they $epsilon$-cross by $min(y_1 (alpha), y_1 (beta))$.

  When $epsilon$ is zero, we leave it out: we say that $(alpha, beta)$ cross by $y$ if they $0$-cross by $y$.
]

Note that the definition of $epsilon$-crossing is not symmetric: $(alpha, beta)$ $epsilon$-crossing is
not the same as $(beta, alpha)$ $epsilon$-crossing. We will usually talk about $(alpha, beta)$ $epsilon$-crossing
in the context that $alpha$ starts off to the left of $beta$, and in this context "$(alpha, beta)$ $epsilon$-cross" means
that at some height before the end of $alpha$ and $beta$, $alpha$ has definitely crossed to the right of $beta$.

== Partially ordered sweep-lines

At our first pass, we won't try to detect intersections at all. Instead, we'll produce
a continuum of sweep-lines (constant except at a finite number of points) that *approximately*
track the horizontal order of the segments.

#def[
The ordered collection $(alpha^1, ..., alpha^m)$ of line segments is #emph[$epsilon$-ordered at $y$]
if each $alpha^i$ has $y$ in its domain and $alpha^i prec.tilde_(y,epsilon) alpha^j$ for all $1 <= i < j <= m$.
]

Our algorithm will produce a family of sweep-lines that are $epsilon$-ordered at every $y$
(and also the sweep-lines will be #emph[complete] in the sense that the sweep-line at $y$ will contain all line segments whose
domain contains $y$). This seems weaker than finding all the intersections (for example, because if you
find all intersections you can use them to produce a completely ordered family of sweep-lines), but
in fact they're more-or-less equivalent: given weakly-ordered sweep-lines, you can perturb the lines so
that your weak order becomes the real order of the perturbed lines.

#lemma[
If $(alpha^1, ..., alpha^m)$ is $epsilon$-ordered at $y$ then there exist $x^1 <= ... <= x^m$ such that
$alpha_(-,epsilon)^i (y) <= x^i <= alpha_(+,epsilon)^i (y)$ for all $i$.
]<lem-horizontal-realization>

#proof[
Define $x^i = max_(j <= i) alpha_(-,epsilon)^j (y)$.
Since we've included $j = i$, this clearly satisfies $x^i >= alpha_(-,epsilon)^j (y)$.
For the other inequality, the ordering condition implies that $alpha^j_(-,epsilon) (y) <= alpha^i_(+,epsilon) (y)$
for every $j <= i$. Therefore this inequality still holds for the maximum over these $j$. 
]

One consequence of our approximate approach is that we need to do a little extra bookkeeping to maintain
the continuity of the input paths: when one segment exits and its path-neighbor enters, we need to remember
that they are connected because the approximate sweep-line might not keep them together. We'll ignore this
bookkeeping for now;
the goal here is to get into detail about the sweep-line invariants, and prove that we can maintain them.

== The sweep-line invariants

We're going to have a sweep-line that depends on $y$. When we need to emphasize this, we'll use the
cumbersome but explicit notation
$(alpha_y^1, ..., alpha_y^(m_y))$.
In addition to the sweep-line, we'll maintain a queue of "events." The events are ordered by $y$-position
and they are similar to the classical Bentley-Ottmann events so we'll skimp on the details here. There
is an "enter" event, an "exit" event, and an "intersection" event.

Our sweep-line will maintain two invariants:
+ At every $y$, the sweep-line is $epsilon$-ordered at $y$. (We'll call this the "order" invariant.)
+ For every $y$ and every $1 <= i < j <= m_y$, if $alpha_y^i$ and $alpha_y^j$ $epsilon$-cross
  then the event queue contains an event between $i$ and $j$,
  and at least one of these events occurs before the $epsilon$-crossing height, or at $y$.
  (We'll call this the "crossing" invariant.)

(When we say the event queue contains an event between $i$ and $j$, we mean that either there's
an exit event for some $alpha^k$ with $i < k < j$ or there's an intersection event for some $alpha^k$ and $alpha^ell$
with $i <= k < ell <= j$.)

Hopefully the first invariant is already well-motivated, so let's discuss the second.
To naively ensure that we find
all intersections, we could adopt a stronger crossing invariant that requires all pairs of intersecting segments
in the sweep-line to immediately put an intersection event in the queue. This would be inefficient to maintain because
it would require testing all pairs of segments. The main Bentley-Ottmann observation is that it's enough to have intersection
events for all sweep-line-adjacent pairs, because any pair of lines will have to become sweep-line adjacent strictly
before they intersect. We can't rely only on sweep-line adjacency because of the $epsilon$ fudge, but our "crossing"
event essentially encodes the same property. Note that if $j = i+1$ and $y$ is just before the $epsilon$-crossing height and
there are no other segments nearby, then the mysterious $j'$ must be either $i$ or $j$ (because there is nothing in between)
and the mysterious queue event must be the crossing of $i$ and $j$ (it couldn't be an exit event, because we assumed they
cross and they must cross before they exit). In other cases, the crossing event ensures that even if we haven't
recorded the upcoming crossing of $alpha^i$ and $alpha^j$, something will happen in between them that will give us
a chance to test their crossing.

== Sweeping the sweep line

The first observation is that the sweep line invariants are maintained as we increase $y$ continuously up
to the next event:
+ For the order invariant, first note that there is an event whenever $y$ leaves the domain of a segment, so $y$ remains in all domains until the next event.
  Moreover, if at any $y' > y$ the ordering breaks, two line segments must have $epsilon$-crossed one another by $y'$.
  The third invariant guarantees that there's an event before this happens, so by the contra-positive until an event happens the ordering
  constraint is maintained.
+ The crossing invariant is maintained because the set of things to check (i.e. the set of line segments that $epsilon$-cross
  one another after $y$) only shrinks as $y$ increases.

== Interaction with adjacent segments

In vanilla Bentley-Ottmann, each segment gets compared to its two sweep-line neighbors; they can either intersect or not intersect.
When numerical errors are taken into account, we may need to compare to
more segments.

#figure(
cetz.canvas({
  import cetz.draw: *

  let calc_eps(x0, x1) = 0.5 * calc.sqrt(1 + calc.pow(calc.abs(x1 - x0) / 2, 2))

  let mkline(x0, x1) = {
    let eps = calc_eps(x0, x1)
    line((x0, 1), (x1, -1))
    line((x0 + eps, 1), (x1 + eps, -1), stroke: ( thickness: 0.2pt, dash: "dashed" ))
    line((x0 - eps, 1), (x1 - eps, -1), stroke: ( thickness: 0.2pt, dash: "dashed" ))
  }

  let a0 = -8
  let a1 = 8
  let b0 = -2
  let b1 = 2
  let c0 = 0.5
  let c1 = 0.5

  content((a0, 1), $alpha^1$, anchor: "south", padding: 5pt)
  content((b0, 1), $alpha^2$, anchor: "south", padding: 5pt)
  content((c0, 1), $alpha^3$, anchor: "south", padding: 5pt)

  mkline(a0, a1)
  mkline(b0, b1)
  mkline(c0, c1)

  let a_eps = calc_eps(a0, a1)
  let b_eps = calc_eps(b0, b1)
  let c_eps = calc_eps(c0, c1)
  let cross12 = 1 - 2*(b0 + b_eps - a0 + a_eps) / (b0 - a0 - (b1 - a1))
  let cross13 = 1 - 2*(c0 + c_eps - a0 + a_eps) / (c0 - a0 - (c1 - a1))

  line((-10, cross13), (10, cross13), stroke: ( thickness: 0.2pt ))
  line((-10, cross12), (10, cross12), stroke: ( thickness: 0.2pt ))
}),
caption: [Three segments all mixed up. The upper horizontal line is the height at which $(alpha^1, alpha^3)$ $epsilon$-cross,
and the lower horizontal line is the height at which $(alpha^1, alpha^2)$ $epsilon$-cross.]
)<mixed-up-segments>

For example, in @mixed-up-segments imagine that $alpha^1$ and $alpha^2$
are present in the sweep line, and we just added $alpha^3$. If we compare
$alpha^3$ to $alpha^2$, we'll see that the $epsilon$-cross and so we'll add an
intersection event between them sometime betfore they $epsilon$-cross. But if
that's all we do then we're in trouble, because by the time we come around to
looking at that intersection event $alpha^1$ may have already $epsilon$-crossed
$alpha^3$, breaking the ordering invariant. Note that $alpha^2$ doesn't
$epsilon$-cross $alpha^3$ at all in @mixed-up-segments, so there's no guarantee of
an intersection event between those two.

Fix $y$ and $epsilon$, and
suppose we have a collection of lines $(alpha^i, ..., alpha^n)$ that satisfy the ordering invariant
at $y$, and suppose also that $(alpha^(i+1), ..., alpha^n)$
satisfy the crossing invariant at $y$.
To make the whole collection satisfy both invariants, we run the following algorithm.
We call this an *intersection scan to the right*.

#figure(
  pseudocode-list([
    + *for* $j = i+1$ up to $n$
      + #line-label(<w-def>) let $w^j$ be the smallest height of any event between $i$ and $j$

      + #line-label(<crossing-test>) *if* $(alpha^i, alpha^j_(+,epsilon))$ cross by $w^j$
        + choose $z$ before the crossing, such that $alpha^i approx_(z,epsilon) alpha^j$
        + insert an intersection event for $(alpha^i, alpha^j)$ at $z$

      + #line-label(<protect>) *if* $alpha^i (z) <= alpha^j_-(z)$ for all $z in [y, w^j]$
        + *break*
  ]),
  caption: "Intersection scan to the right"
)

#inexact[
The test at @protect can be seen as an early-stopping optimization, and is not necessary for correctness.
In particular, an approximation with no false positives
is also fine.
]

#lemma[
Suppose that $(alpha^i, ..., alpha^n)$ satisfy the ordering invariant
at $y$, and suppose also that $(alpha^(i+1), ..., alpha^n)$
satisfy the crossing invariant at $y$.
After running an intersection scan to the right,
$(alpha^i, ..., alpha^n)$ satisfy the crossing invariant at $y$.

In fact, $(alpha^i, ..., alpha^n)$ satisfy a slightly stronger crossing invariant at $y$: for every $j > i$,
if $(alpha^i, alpha^j_(+,epsilon))$ cross then the event queue contains an event between $i$ and $j$, and before
 the crossing height.
]<lem-intersection-scan>

(The special thing about the stronger crossing invariant is that it asks whether
$(alpha^i, alpha^j_(+,epsilon))$ cross, where the usual crossing invariant asks
whether 
$(alpha^i_(-,epsilon), alpha^j_(+,epsilon))$ cross.)

#proof[
  It suffices to check the stronger crossing invariant.
  So take some $k > i$
  that $(alpha^i, alpha^k_(+,epsilon))$ cross. We consider two cases: whether or not the loop terminated
  before reaching $k$.

  - Suppose the loop terminated at some $j < k$. If $(alpha^i, alpha^k_(+,epsilon))$ cross
    after $w^j$, then the definition of $w^j$ ensures that there is an event between $i$ and $j$ (and therefore between
    $i$ and $k$) before the crossing. On the other hand, the termination condition ensures that
    $alpha^i (z) <= alpha^j_-(z)$ until $w^j$, and so if $(alpha^i, alpha^k_(+,epsilon))$ cross before $w^j$ then
    also $(alpha^j, alpha^k)$ cross before that. In this case, the crossing invariant for $(alpha^(i+1), ..., alpha^n)$
    implies the existence of an event between $j$ and $k$ (and therefore between $i$ and $k$) before the crossing.
  - If the loop included the case $j = k$, we break into two more cases:
    - If $(alpha^i, alpha^k_(+,epsilon))$ cross by $w^j$, then the algorithm inserted a witnessing event between $i$ and $j$.
    - Otherwise, the definition of $w^j$ ensures that there is an event between $i$ and $j$ before the crossing.
]

#remark[
We can tweak the algorithm a little to try and reduce the number of comparisons.
For a start, if we add an intersection event between $i$ and $j$ then we can
set $w^j$ to $z$, the height of that new event (because $z$ is the new smallest
height of any event between $i$ and $j$).
Therefore, if we broaden the test @crossing-test to check whether $(alpha^i,
alpha^j_-)$ cross, and we choose $z$ to be before the crossing of $(alpha^i,
alpha^j_-)$ then we can skip the test at @protect and terminate straight away.
Thus, there is no loop at all: we only need to consider $j = i + 1$.

One potential issue with this is finding a height $z$ with $alpha^i approx_(z,epsilon) alpha^j$
that's before $(alpha^i, alpha^j_-)$ cross; in other words, we want a height after $(alpha^i_+, alpha^j_-)$
cross but before $(alpha^i, alpha^j_-)$ cross. If $alpha^j$ is almost horizontal, this window might be very
small (or maybe even not representable with our restricted precision).
It may still be worthwhile to have a fast path with a single comparison, and a slow path with a loop.
]

As you might have already guessed, we can also intersection scan to the left; it's pretty much a reflection
of the other one.

#figure(
  pseudocode-list([
    + *for* $j = i$ down to $1$
      + let $w^j$ be the smallest height of any event between $j$ and $i$

      + *if* $(alpha^j_(-,epsilon), alpha^i)$ cross by $w^j$
        + choose $z$ before the crossing, such that $alpha^j approx_(z,epsilon) alpha^i$
        + insert an intersection event for $(alpha^j, alpha^i)$ at $z$

      + *if* $alpha^j_+ (z) <= alpha^i (z)$ for all $z in [y, w^j]$
        + *break*
  ]),
  caption: "Intersection scan to the left"
)

We'll skip the proof of the following lemma, because it's basically the same as the last one.

#lemma[
Suppose that $(alpha^1, ..., alpha^i)$ satisfy the ordering invariant
at $y$, and suppose also that $(alpha^1, ..., alpha^(i+1))$
satisfy the crossing invariant at $y$.
After running an intersection scan to the left,
$(alpha^1, ..., alpha^(i+1))$ satisfy a slightly stronger crossing invariant at $y$: for every $j <= i$,
if $(alpha^j_(-,epsilon), alpha^(i+1))$ cross then the event queue contains an event between $j$ and $i+1$, and before
 the crossing height.
]<lem-intersection-scan-left>

The purpose of the stronger crossing invariant comes in the next lemma, which deals with scanning in both directions
and allows the insertion of a segment in the middle of a sweep-line.

#lemma[
Suppose that $(alpha^1, ..., alpha^n)$ satisfy the ordering invariant at $y$, and suppose that
$(alpha^1, ..., alpha^i)$ and $(alpha^(i+1), ..., alpha^n)$ satisfy the crossing invariant at $y$.
Let $beta$ be another segment and assume that $(alpha^1, ..., alpha^i, beta, alpha^(i+1), ... alpha^n)$
satisfy the ordering invariant at $y$. After running an intersection scan to the left and an intersection
scan to the right from $beta$, 
$(alpha^1, ..., alpha^i, beta, alpha^(i+1), ... alpha^n)$ satisfies the crossing invariant at $y$.
]<lem-intersection-scan-bidirectional>

#proof[
@lem-intersection-scan implies that $(beta, alpha^(i+1), dots, alpha^n)$ satisfies the crossing invariant,
and
@lem-intersection-scan-left implies that $(alpha^1, ..., alpha^i, beta)$ does also. It only remains
to consider interactions between a segment before $beta$ and one after. So fix $j <= i < k$ and suppose
that $(alpha^j, alpha^k)$ $epsilon$-cross. If they $epsilon$-cross after $y_1(beta)$ then $beta$ exit
event witnesses the crossing invariant, so assume that $(alpha^j, alpha^k)$ $epsilon$-cross by $y_1(beta)$.
Then $(alpha^j_-, alpha^k_+)$ cross by $y_1(beta)$, and so one of them crosses $beta$ before $(alpha^j, alpha^k)$ $epsilon$-cross.
If $(alpha^j_-, beta)$ cross then @lem-intersection-scan-left implies that there is an event between $alpha^j$ and $beta$ (and
therefore between $alpha^j$) and $alpha^k$ before the crossing height; otherwise, $(beta, alpha^k_+)$ cross
and so @lem-intersection-scan provides the required event.
]

One last observation in this section, that follows trivially from the algorithm:

#lemma[
If an intersection scan inserts an intersection event for $(alpha, beta)$
then the intersection event's height $z$ satisfies $alpha approx_(z,epsilon) beta$.
]<lem-valid-intersection-events>

== An "enter" event

When inserting a new segment into the current sweep-line, we first choose its sweep-line position using
a binary search on its horizontal coordinate. Let's write $(alpha^1, dots, alpha^m)$ for the sweep-line
before inserting the new segment, and let's call the new segment $beta$. First, we observe that
there is a place to insert the new segment while preserving the ordering invariant.

#lemma[
Suppose $(alpha^1, ..., alpha^m)$ is $epsilon$-ordered at $y$, and let $i$ be the largest $j$ for which
$alpha^j prec_(y,epsilon) beta$. Then
$(alpha^1, ..., alpha^i, beta, alpha^(i+1), ..., alpha^m)$ is $epsilon$-ordered at $y$.
(Here, we can allow the corner cases $i = 0$ and $i = m$ by declaring that
"$alpha^0$" is a vertical line at $x = -infinity$ and "$alpha^(m+1)$" is a vertical line at $x = infinity$).
]<lem-insert-preserving-order>

#proof[
Since $(alpha^1, ..., alpha^m)$ was $epsilon$-ordered at $y$, it suffices to compare beta with all $alpha^j$.
For $i + 1 <= j <= m$, our choice of $i$ immediately implies that $alpha^j succ.tilde_(y,epsilon) beta$.
So consider $1 <= j <= i$. Since $(alpha^1, ..., alpha^m)$ is $epsilon$-ordered, $alpha^j prec.tilde_(y,epsilon) alpha^i$.
Since $alpha^i prec_(y,epsilon) beta$, @lem-basic-order-properties implies that $alpha^j prec.tilde_(y,epsilon) beta$.
]

#inexact[
@lem-insert-preserving-order guarantees the existence of an insertion point, but it doesn't say how to
find it efficiently.
But consider a predicate $f(alpha^j)$ that returns true whenever $alpha^j prec_(y,epsilon) beta$, and
false whenever $alpha^j_+(y) > beta_+(y)$. Running a binary search with this predicate will find some $i$
for which $f(alpha^i)$ is true and $f(alpha^(i+1))$ is false. By scanning to the right from there, we can
find the largest such $i$.

This $i$ is at least as large as the $i$ in @lem-insert-preserving-order, so
to check that it's a valid insertion point we only need to check that it isn't too large. So if $1 <= j <= i$ then
$alpha^j prec.tilde_(y,epsilon) alpha^i$ and so $alpha^j_-(y) <= alpha^i_+(y)$. On the other hand, $f(alpha^i)$
was true and so $alpha^i_+ <= beta_+(y)$. Putting these together shows that $alpha^j prec.tilde_(y,epsilon) beta$.

This algorithm can be implemented with approximate arithmetic, and its running time is logarithmic in the
total length of the sweep line, plus linear in the number of elements that are very close to $beta$.
]

@lem-insert-preserving-order implies that we can insert a new segment while preserving the ordering invariant. By
@lem-intersection-scan-bidirectional, running an intersection scan restores the crossing invariant.
Thus, we can insert a new segment while preserving the sweep-line invariants.

== An "exit" event

When a segments exits the sweep-line, the ordering invariant clearly doesn't break.
Regarding the crossing invariant, it can only break because of $epsilon$-crossing pairs whose
crossing invariant was witnessed by the exit event that was just processed.
To restore the crossing invariant, we need to enqueue some new intersection events.

Let $(alpha^1, ..., alpha^m)$ be the sweep-line after removing the just-exited segment
which, we assume, used to live between $alpha^i$ and $alpha^(i+1)$. Note that both
$(alpha^1, ..., alpha^i)$ and $(alpha^(i+1), ..., alpha^n)$ satisfy the crossing invariant.
By @lem-intersection-scan-bidirectional, running an intersection scan from $alpha^i$ in both
directions restores the crossing invariant. (Technically, this isn't covered by the statement
of @lem-intersection-scan-bidirectional -- which involves a new segment $beta$ -- but the proof
is basically the same.)

== An "intersection" event

Suppose our sweep-line is $(alpha^1, ..., alpha^m)$ and we've just encountered an intersection event for $(alpha^i, alpha^j)$
at height $y$.
If $i > j$ then they've already been swapped in our sweep-line, so we don't need to swap them again. If $i < j$, we need to
swap them. According to @lem-valid-intersection-events, $alpha^j prec.tilde_(y,epsilon) alpha^i$. It seems reasonable, therefore,
to reorder the sweep line by putting $alpha^i$ after $alpha^j$, like

$
alpha^1, ..., alpha^(i-1), alpha^(i+1), ..., alpha^j, alpha^i, alpha^(j+1), ... alpha^n.
$

The issue with this is that $prec.tilde_(y,epsilon)$ is that it's changed the order of pairs other than $alpha^i$ and $alpha^j$:
for every $i < k < j$, the ordering between $alpha^i$ and $alpha^k$ has been swapped. If $prec.tilde$ were transitive, this would
be fine. But it isn't.

To fix this issue, we allow $alpha^i$ to "push" some of the intermediate segments along with it. It's a bit tedious to write (and read)
this precisely, so hopefully this description is enough: for each $k$ between $i$ and $j$, if $alpha^i prec_(y,epsilon) alpha^k$ then
we also move $alpha^k$ after $alpha^j$. Also, we preserve the relative order of all segments moved in this way.
To see that this "pushing" maintains the ordering invariants, note that by definition it preserves the ordering for all comparisons
with $alpha^i$: if some $alpha^ell$ was pushed along with $alpha^i$ then their relative orders haven't changed; and if $alpha^ell$
wasn't pushed then $alpha^ell prec.tilde_(y,epsilon) alpha^i$ and the new order is fine.

What about other pairs? If $alpha^k$ and $alpha^ell$
changed orders then one of them ($alpha^ell$, say) was pushed and the other wasn't. Then $alpha^i prec_(y,epsilon) alpha^ell$
by our choice of the pushed segments, and $alpha^k prec.tilde_(y,epsilon) alpha^i$ by the previous paragraph. Putting these
together, @lem-basic-order-properties implies that $alpha^k prec.tilde_(y,epsilon) alpha^ell$ and so the new order is ok.

#inexact[
We might not be able to tell exactly which segments $alpha^k$ between $alpha^i$ and $alpha^j$ satisfy
$alpha^i prec_(y,epsilon) alpha^k$. Fortunately, we can push a few too many segments while still maintaining
correctness: it suffices to
include all segments $alpha^k succ_(y,epsilon) alpha^i$, while also ensuring that we only include $alpha^k$ for which
$alpha^k_+(y) >= alpha^i_+(y)$.
]

Finally, we need to maintain the crossing invariant. We can probably be more efficient about this, but one
way to be correct is to treat the various swappings as a bunch of deletions from the sweep-line followed by
a bunch of insertions into the sweep-line. We have already shown that running an intersection scan
after each insertion and deletion correctly maintains the crossing invariant, so we can just do that.

= Correctness of the output

We've described how to turn a bunch of segments into a continuum of sweep-lines, but we probably actually wanted
to find the segments' intersection points. Let's define exactly what that means and how to get there.
We'll assume that we start with one or more polylines that may or may not be closed. Equivalently, each segment
comes with an orientation and (optionally) a "following" segment whose starting point is the ending point of the
current segment.
Our goal is to subdivide all segments at all intersection points, meaning that for each segment we'll produce
a polyline; we'll call the segments in each such polyline "output segments," to distinguish them from
the original segments. We require three properties:

- "approximation": for each input segment, every vertex in its output polyline must be within $epsilon$ of the original segment.
  Also the first and last vertices in the output polyline must be within $epsilon$ of the start and end of the segment, respectively.
- "continuity": if a segment has a following segment, then the last vertex in its output polyline must coincide
  with the first vertex of the following segment's output polyline.
- "completeness": output segments intersect only at their endpoints, unless they are identical.

In the solution we describe, we'll achieve both continuity and the second half
of approximation by insisting that output polylines have exactly the same start-
and endpoints as their input segment. This stronger condition makes the algorithm
simpler, but it also subjectively can hurt output "quality" by requiring more segments.

== The "dense" version

Suppose we've already found a weakly-ordered sweep line at every height. We say
a height is "important" if the sweep line changed at that height, either because
some segments entered or exited, or because the order changed. The naive version
of our subdivision algorithm will subdivide every segment at every important
height. Note that because the sweep line changes at a discrete set of heights,
at every important height $y$ we actually have two weakly-ordered sweep line:
the "old" one from infinitesmally before $y$ height and the "new" one from $y$
onwards. Because everything is continuous and the weak ordering conditions are
closed, both the old and new sweep lines satisfy the weak ordering condi0.2tions
at $y$.

The algorithm goes like this: at every important height we use
@lem-horizontal-realization to assign horizontal positions to segments in the
old sweep line; we subdivide each segment at the resulting coordinate. Then we
use @lem-horizontal-realization again to assign horizontal positions to segments
in the new sweep line. If a segment gets assigned two different horizontal positions
by the old and new sweep lines, we add a horizontal segment between them. Also,
if a segment starts or ends at the current height and its assigned position is not the
same as its starting or ending position, we fix that up with another horizontal segment.
Finally, we subdivide all horizontal segments as necessary to ensure that they only
intersect at endpoints.

I think it's pretty clear that this algorithm is correct. The approximation property
holds (with $sqrt 2 epsilon$ instead of $epsilon$, but that doesn't matter) because
@lem-horizontal-realization always puts a new point within its segment's upper and lower bounds,
and the bounds are chosen to be within $sqrt 2 epsilon$ of the segment; see @fig-error-bars.
As mentioned above, the "continuity" property and the other half of "approximation" follow
because we insisted on including the segment endpoints exactly.

For the "completeness" property, let's focus on the bit between the important heights
(because everything gets subdivided at the important heights, and so maintaining the
completeness property there is just a matter of splitting into enough horizontal segments).
Between two important heights, the sweep line is constant, so the new sweep line
at the old height is the same as the old sweep line at the new height. Thus, all output
subsegments have the same order when leaving the old sweep line as they do when entering
the new sweep line. Therefore any two of them that intersect between the two important
heights must be identical.

== The "sparse" version

We'd like to avoid subdividing every segment at every important height. This basically
involves detecting which segments need to be divided and ignoring the rest. It's implemented
but not yet written up (TODO).

== A more detailed insertion algorithm

It took me a few tries to get the approximate insertion algorithm working, so
I figured it was worth writing up a few more details. First of all, a slight
tweak of @lem-insert-preserving-order shows that there is a position $i$ such
that $alpha^j_- (y) <= beta(y)$ for all $j <= i$ and $alpha^j_+ (y) >= beta(y)$
for all $j > i$. We will look for this $i$, but with some slight slack in our
arithmetic. The slack will be small enough to guarantee we find a position that
satisfies the weaker properties of @lem-insert-preserving-order.

First, we consider a predicate $p(j)$ that returns `false` whenever $alpha^j_+
(y) > beta(y)$. We put no conditions on the values of $j$ for which $p(j)$
returns `true`; a tighter predicate will give a more efficient algorithm, but
doesn't affect correctness. Let $i_-$ be such that $p(i_-) = #true$ and $p(i_-
+ 1) = #false$ (where we allow $i_- = -1$, and handle the boundary cases by
declaring that $p(-1) = #true$ and $p(m+1) = #false)$. Note that $i_-$ can be
found with a binary search.

The definition of $p$ ensures that $alpha^(i_-)_+(y) <= beta(y)$. Under the
assumption that $(alpha^1, ..., alpha^m)$ is weakly ordered, it follows that
for every $j <= i_-$, $alpha^j_-(y) <= beta(y)$. That is, the $i$ that we are
looking for is larger than or equal to $i_-$.

The next step is a linear scan up from $i_-$. Let $q(j)$ be a predicate
that returns `true` whenever $alpha^j_- (y) < beta^j (y)$ and `false` whenever
$alpha^j_- (y) > beta^j_+ (y)$. Take $i >= i_-$ to be the index just before $q$
first returns false; we will insert $beta$ after $alpha^i$.

Let's check that this insertion position is valid: for every $j <= i$ we have
either $j <= i_-$ or $i_- < j <= i$. In the first case, we already observed
that $alpha^j_-(y) <= beta(y)$; in the second case, $q(j)$ returned `true` and
so $alpha^j_-(y) <= beta^j^+ (y)$. In either case, $alpha^j prec.tilde beta$.
On the other hand, $q(i+i)$ being `false` implies that $alpha^(i+1)_- (y) >=
beta(y)$. This implies that for all $j >= i+1$, $alpha^j_+ (y) >= beta(y)$ and
so $alpha^j succ.tilde_y beta$.

= Accuracy analysis

We're going to implement some algorithms in floating point. Suppose $p$ is the number of mantissa bits
of the floating-point type, and let $fl(t)$ denote the rounding function. Let $epsilon_0 = 2^(-1-p)$;
this is the relative error of the rounding function for all normal numbers: for a real number $x$,
as long as $fl(x)$ is finite and non-subnormal,
$
fl(x) = x plus.minus epsilon_0 |x|.
$
Now, if the result of floating point addition or subtraction is subnormal then it is exact; therefore,
for any floating-point $x$ and $y$, if $x - y$ and $x + y$ don't overflow then after rounding they
have relative error at most $epsilon_0$.

#lemma[
  If $y_0 <= y <= y_1$ are floating-point numbers and $epsilon_0 <= 1/2$ then computing $(y - y_0) / (y_1 - y_0)$
  the naive way gives an absolute error of
  at most $3 epsilon_0 + 4 epsilon_0^2$.
]<lem-ratio-error>

#proof[
The assumption $y_0 <= y <= y_1$ ensures that $y - y_0$ and $y_1 - y_0$ do not overflow. Therefore

$
fl(y - y_0) &= (y - y_0) (1 plus.minus epsilon_0) \
fl(y + y_0) &= (y + y_0) (1 plus.minus epsilon_0)
$
and so
$
fl(y - y_0) /
fl(y_1 - y_0) = (y - y_0) / (y_1 - y_0) times
 (1 plus.minus epsilon_0) / (1 plus.minus epsilon_0).
$
For the upper bound,
$
 (1 + epsilon_0) / (1 - epsilon_0) = (1 + epsilon_0) (1 + epsilon_0 + epsilon_0^2 / (1 - epsilon_0))
= 1 + 2 epsilon_0 + 2 / (1 - epsilon_0) epsilon_0^2 <= 1 + 2 epsilon_0 + 4 epsilon_0^2.
$
For the lower bound,
$
 (1 - epsilon_0) / (1 + epsilon_0) = 1 - epsilon_0 - (epsilon_0 (1-epsilon_0)) / (1+epsilon_0) >= 1 - 2 epsilon_0.
$
Putting these together,
$
fl(y - y_0) /
fl(y_1 - y_0) = (y - y_0) / (y_1 - y_0) (1 plus.minus (2 epsilon_0 + 4 epsilon_0^2)).
$
Since $0 <= (y - y_0) / (y_1 - y_0) <= 1$, this relative error is also an absolute error:
$
fl(y - y_0) /
fl(y_1 - y_0) = (y - y_0) / (y_1 - y_0) plus.minus (2 epsilon_0 + 4 epsilon_0^2).
$
The final rounding step adds another absolute error of at most $epsilon_0$.
]

#lemma[
Suppose that $x_0$, $x_1$, and $M$ are floating-point numbers with $|x_0|, |x_1| <= M/2$.
Let $0 <= t <= 1$ be a real number, and let $0 <= hat(t) <= 1$ be a floating point number
with $|t - hat(t)| <= epsilon$. Then computing $x_0 + t (x_1 - x_0)$ the obvious way gives an
absolute error of at most $(3 epsilon_0 + epsilon) M$.
]<lem-convex-combination-error>

#proof[
Let's first consider the error introduced by approximating $t (x_1 - x_0)$ with $hat(t) fl(x_1 - x_0)$:
$
hat(t) fl(x_1 - x_0) = hat(t) (x_1 - x_0)(1 plus.minus epsilon_0) = hat(t) (x_1 - x_0) plus.minus epsilon_0 M,
$
where the second approximation follows because $0 <= hat(t) <= 1$ and $|x_1 - x_0| <= M$.
Since $hat(t) = t plus.minus epsilon$, we continue with
$
hat(t) fl(x_1 - x_0)
= (t plus.minus epsilon) (x_1 - x_0) plus.minus epsilon_0 M,
= t (x_1 - x_0) plus.minus (epsilon_0 + epsilon) M.
$
Rounding the left hand side introduces a relative error of at most $epsilon_0$, which is an absolute error of at most $M epsilon_0$. Therefore,
$
fl(hat(t) fl(x_1 - x_0))
= t (x_1 - x_0) plus.minus (2 epsilon_0 + epsilon) M.
$

Finally, we subtract both sides from $x_0$ and round one more time. This last rounding introduces a relative error of at most $epsilon_0$,
which again leads to an absolute error of at most $2 epsilon_0 M$ (and I think maybe the 2 is unnecessary, but let's not worry).
]

Next, let's analyze the computation of the $y$ intersection point.

#lemma[
Suppose we have two segments $alpha$ and $beta$. Set $y_0 = y_0(alpha) or y_0(beta)$ and $y_1 = y_1(alpha) and y_1(beta)$
(these can be computed exactly). Suppose that we can compute $alpha(y)$ and $beta(y)$ to additive accuracy $epsilon$,
and assume that $beta(y_0) - alpha(y_0) >= 0$ and
$
alpha(y_1) - beta(y_1) > 16/3 (epsilon + 2 epsilon_0 M).
$
Define $t = (alpha(y_1) - beta(y_1)) / (alpha(y_1) - beta(y_1) + beta(y_0) - alpha(y_0)$ and set $y = y_0 + t(y_1 - y_0)$. The
natural way to compute $y$ produces an approximation $hat(y)$ satisfying
$
|alpha(hat(y)) - beta(hat(y))| <= 4(epsilon + 2 epsilon_0 M) (1 + max(1/m_alpha, 1/m_beta)),
$
where $m_alpha$ and $m_beta$ are the slopes of $alpha$ and $beta$.
]<lem-intersection-height-error>

Basically, this says we can find a good approximate crossing height as long as the two segments cross by at least $16/3 (epsilon + 2 epsilon_0 M)$.

#proof[
First, note that at least one of $alpha(y_0)$ or $beta(y_0)$ is calculated exactly; similarly for $alpha(y_1)$ and $beta(y_1)$.
Therefore the absolute error in calculating $alpha(y_1) - beta(y_1)$ is at most $epsilon + epsilon_0 M$ (where the $epsilon_0 M$ comes
from rounding after the summation) and the absolute error in calculating
$(alpha(y_1) - beta(y_1)) + (beta(y_0) - alpha(y_0))$ is at most $2 epsilon + 3 epsilon_0 M$.
These errors aren't independent, though, since both terms involve a computation of $alpha(y_1) - beta(y_1)$.
Specifically, let $c = alpha(y_1) - beta(y_1)$ and let $d = (alpha(y_1) - beta(y_1)) + (beta(y_0) - alpha(y_0))$; suppose
our calculation of $c$ has error $epsilon_c$ and our calculation of $d$ has error $epsilon_d$. Then

$
|epsilon_c - epsilon_d| <= epsilon + 2 epsilon_0 M,
$ <eq-error-cancellation>

because this is the *additional* error introduced into $d$ by computing $beta(y_0) - alpha(y_0)$ and adding it to the other term.

Now we consider the quotient

$
(c + epsilon_c) / (d + epsilon_d)
&= c / (d (1 + epsilon_d/d)) + epsilon_c / (d (1 + epsilon_d/d)) \
&= c / d (1 - epsilon_d/d + (epsilon_d/d)^2 / (1 + epsilon_d/d)) + epsilon_c / d (1 - (epsilon_d/d)/(1 + epsilon_d / d)).
$

We've assumed that $1 + epsilon_d/d >= 1/2$, and so the error
$
abs((c + epsilon_c) / (d + epsilon_d) - c/d)
$
is at most
$
& abs(c / d ( - epsilon_d/d + (epsilon_d/d)^2 / (1 + epsilon_d/d)) + epsilon_c / d (1 - (epsilon_d/d)/(1 + epsilon_d / d))) \
& <= abs(- c / d epsilon_d/d + epsilon_c / d ) + 2 c/d (epsilon_d/d)^2 + 2 abs(epsilon_c epsilon_d) / d^2 \
& <= c/d abs(epsilon_c/d - epsilon_d/d) + (1 - c/d) abs(epsilon_c/d) + 2 (epsilon_d/d)^2 + 2 abs(epsilon_c epsilon_d) / d^2,
$
where in the last line we recall that $0 <= c/d <= 1$. Now, recall from @eq-error-cancellation and the paragraph before it
that both $abs(epsilon_c)$ and $abs(epsilon_c - epsilon_d)$ are bounded by $epsilon + 2 epsilon_0 M$, and that
$abs(epsilon_d)$ is bounded by $2 epsilon + 3 epsilon_0 M$. Therefore, our computation of $c/d$ (which, recall, we called $t$ above) has absolute error of at most

$
(epsilon + 2 epsilon_0 M)/d + 4 (2 epsilon + 3 epsilon_0 M)^2 / d^2.
$

Let's call this expression $epsilon_t$.

By @lem-convex-combination-error, the computation of $y = y_0 + t (y_1 - y_0)$ has absolute error at most $(2 epsilon_0 + epsilon_t) |y_1 - y_0| + epsilon_0 M$.
Let's see what this means for the horizontal accuracy of $alpha(y)$ and $beta(y)$. We can write
$
alpha(y) = alpha(y_0) + (y - y_0) / (y_1 - y_0) (alpha(y_1) - alpha(y_0)),
$
and similarly for $beta(y)$. Combining these and recalling our definitions of $c$ and $d$,
$
alpha(y) - beta(y) = -c + (y - y_0) / (y_1 - y_0) d.
$
For the true value of $y$, this is zero of course. With our error bound of $(2 epsilon_0 + epsilon_t) |y_1 - y_0| + epsilon_0 M$ on our
approximation of $y$ (let's call it $hat(y)$),
$
abs(alpha(hat(y)) - beta(hat(y))) <= (2 epsilon_0 + epsilon_t) d + (epsilon_0 M) / (y_1 - y_0) d
<= epsilon_t d + 3 epsilon_0 (M d) / (y_1 - y_0).
$

If we assume that $16 (epsilon + 2 epsilon_0 M) / d <= 3$ then $epsilon_t d <= 4 epsilon + 8 epsilon_0 M$. 
Since $d/(y_1 - y_1)$ is the difference of the inverse-slopes of $alpha$ and $beta$, it's bounded by twice
the maximum of these inverse-slopes.
]

Putting all this together (and handling the "chamfers" yet):
#lemma[
Assume all line segments are inside the square $[-M/2, M/2]^2$,
and take $delta = 64 epsilon_0 M$. If $epsilon_0 <= 1/4$ then

- We can compute $x$ coordinates of line segments to additive accuracy $delta/8$.
- If $alpha$ $(3 delta)/4$-crosses $beta$, we can compute a crossing height $hat(y)$ such
  that
  $
  |alpha(hat(y)) - beta(hat(y))| <= (9 delta) /16 (1 + max(1/m_alpha, 1/m_beta)).
  $
]

#proof[
We compute $x$ coordinates by first computing $t = (y - y_0) / (y_1 - y_0)$ (with error at most $3 epsilon_0 + 4 epsilon_0^2$ by
@lem-ratio-error). Then we apply @lem-convex-combination-error with $epsilon = 3 epsilon_0 + 4 epsilon_0^2$ to see that we
can compute the horizontal coordinate with error at most $(6 epsilon_0 + 4 epsilon_0^2)M$. If $epsilon_0 <= 1/4$ as we assumed,
this is at most $7 epsilon_0 M <= delta/8$. This proves the first claim.

For the second claim, we apply @lem-intersection-height-error with $epsilon = (6 epsilon_0 + 4 epsilon_0^2) M$.
With this $epsilon$, $epsilon + 2 epsilon_0 M) <= 9 epsilon_0 M$ and so $16/3 (epsilon + 2 epsilon_0 M) <= 48 epsilon_0 M = (3 delta)/4$.
Thus, if $alpha$ $(3 delta)/4$-crosses $beta$ then the assumptions of @lem-intersection-height-error are satisfied
(and in the conclusion, $4 (epsilon + 2 epsilon_0) M <= 36 epsilon_0 M = (9 delta) /16 $.
]

= Curves etc.

Let's move on from lines and talk about curves. Each curve is of the form $alpha: [0, 1] -> bb(R)^2$, which we'll sometimes
in components as $alpha_0: [0, 1] -> bb(R)$ and $alpha_1: [0, 1] -> bb(R)$. We'll assume that each $alpha_0$ is a strictly
increasing function. Write $S = [-1, 1] times [-1, 1]$ for the unit square in $bb(R)^2$. For a curve $alpha$ and a set $B subset bb(R)^2$, we write
$alpha + B$ for the set

$
alpha + B = {(x, y) in bb(R)^2 : exists t in [0, 1] "with" (x, y) - alpha(t) in B}
$

In other words, $alpha + B$ is the Minkowski sum between $B$ and the locus of $alpha$. For any set $B subset bb(R)^2$,
let

$
B_y = {x in bb(R) : (x, y) in B}.
$

For example, here is a picture of $(alpha + epsilon B)_y$: (TODO)

== The curve orders

When moving to curves, it seems useful to work with more abstract orders. For fixed $epsilon$ and $y$, we'll define
three of them: the "weak pre-order," the "strong order," and the "weak order."

#let weakprelt = sym.lt.tri
#let weakpreleq = sym.lt.eq.tri
#let notweakprelt = sym.lt.tri.not
#let weakpreeq = sym.eq.dots
#let weaklt = sym.lt
#let stronglt = sym.lt.double
#let weaklt = sym.lt
#let weakeq = sym.tilde

We begin with the weak pre-order.

#def[
  For each $epsilon >= 0$ and $y in bb(R)$, let $weakprelt_(y,epsilon)$ be a relation satisfying:

  - if $alpha weakprelt_(y,epsilon) beta$ then $(alpha + epsilon/2 S)_y <= (beta + epsilon/2 S)_y$,
  - if $(alpha + epsilon S)_y <= (beta + epsilon S)_y$ then $alpha weakprelt_(y,epsilon) beta$,
  - for any $k >= 1$, there are no $alpha_1, ..., alpha_k$ with $alpha_1 weakprelt_(y,epsilon) alpha_2 weakprelt_(y,epsilon) dots.h.c weakprelt_(y,epsilon) alpha_k weakprelt_(y,epsilon) alpha_1$.

  We call this last condition the "no cycles" condition.
]

Note that we *do not* assume transitivity: $alpha weakprelt_(y,epsilon) beta
weakprelt_(y,epsilon) gamma$ does not imply that $alpha weakprelt_(y,epsilon)
gamma$. We also do not assume anti-symmetry: it's possible to have neither
$alpha weakprelt_(y,epsilon) beta$ nor $beta weakprelt_(y,epsilon) alpha$. In
fact, if $(alpha + epsilon/2)_y$ and $(beta + epsilon/2 S)_y$ overlap, it is
guaranteed that neither of these orderings hold.

#def[
  If neither $alpha weakprelt_(y,epsilon) beta$ nor $beta weakprelt_(y,epsilon) alpha$, we say that

  $
  alpha weakpreeq_(y,epsilon) beta.
  $

  If either $alpha weakprelt_(y,epsilon) beta$ or 
  $alpha weakpreeq_(y,epsilon) beta$, we write
  $alpha weakpreleq_(y,epsilon) beta$. Equivalently, 
  $alpha weakpreleq_(y,epsilon) beta$ is the negation of 
  $beta weakprelt_(y,epsilon) alpha$.
]

TODO: draw a picture.

The strong order is similar to the weak pre-order, but with bigger epsilons.
#def[
  For each $epsilon >= 0$ and $y in bb(R)$, let $stronglt_(y,epsilon)$ be a relation satisfying:

  - if $alpha stronglt_(y,epsilon) beta$ then $(alpha + 2 epsilon S)_y <= (beta + 2 epsilon S)_y$,
  - if $(alpha + 3 epsilon S)_y <= (beta + 3 epsilon S)_y$ then $alpha stronglt_(y,epsilon) beta$,
  - for any $k >= 1$, there are no $alpha_1, ..., alpha_k$ with $alpha_1 stronglt_(y,epsilon) alpha_2 stronglt_(y,epsilon) dots.h.c stronglt_(y,epsilon) alpha_k stronglt_(y,epsilon) alpha_1$.
]

The definition of the weak order is a little strange at first. The initial idea
is that we want to ensure that our sweep-line is ordered by the weak pre-order:
we are going to try to ensure that if our sweep-line at $y$ is $alpha^1, ..., alpha^n$
then for all $i < j$, $alpha^i weakpreleq_y alpha^j$. Or in other words, we want to
ensure that whenever $alpha^j$ is definitely to the right of $alpha^i$, it comes after $alpha^i$
in the sweep-line.

For sweep-line algorithms to be efficient, we need to avoid too many segment comparisons.
In the classical Bentley-Ottmann algorithm with exact arithmetic this is easy: whenever
we look for intersections against segment $alpha^i$, we compare it with its sweep-line
neighbors to the left and right (so, $alpha^(i-1)$ and $alpha^(i+1)$ in our notation).
Inexact arithmetic will make this a little trickier; we will typically need to look further
than just the immediate neighbors of $alpha^i$. However, we will need *some* mechanism
for early stopping, some way to say that after we have compared $alpha^i$ to, say, $alpha^(i-k)$
through $alpha^(i-1)$, we can be sure that no other $alpha^j$ (for $j < i - k$) needs
to be checked. The intuitive idea is that if $alpha^(i-k)$ is very far to the left of $alpha^i$
then we can stop, because then $alpha^(i-k)$ will "shield" $alpha^i$ from segments further
left, in much the same way that its immediate neighbors "shield" $alpha^i$ in the classical
sweep-line algorithm.

We would like to use the strong order $stronglt_y$ to check whether $alpha^(i-k)$ is "very far
to the left" of $alpha^i$, but we run into trouble with examples like this:

TODO: picture.

Here, we clearly see that $alpha$ is to the left of $beta$; in situations like this, if we're
examining $beta$ for potential intersections and we see $alpha$ to its left, we'd really like
to stop looking further left. But $gamma$ is a problem: it's almost horizontal, and the
sweep-line at $y$ is at an ambiguous height, being more than $epsilon / 2$ but less than $epsilon$ below $gamma$.
According to the definition of our weak pre-order,
$alpha weakprelt_y gamma$ and $alpha weakpreeq_y gamma$ are
both allowed, as are both
$beta weakprelt_y gamma$ and $beta weakpreeq_y gamma$. So imagine that $alpha weakpreeq_y gamma$ and $beta weakprelt_y gamma$.
Before adding $beta$, the sweep-line could have been $(gamma, alpha)$. Then we could have tried to add $beta$ after $alpha$
(because clearly $beta$ is to the right of $alpha$), leading to the sweep-line $(gamma, alpha, beta)$.
Scanning for any intersections with the newly-added $beta$ won't find any because we'll look to the left, see
$alpha$, and stop. The result of all this is the illegal sweep-line $(gamma, alpha, beta)$, in which
$beta weakprelt_y gamma$ but $gamma$ comes first in the sweep-line.

Basically, it seems hard to have both early stopping (which we want for efficiency) and the
ordering invariants that we need for correctness. Our "fix" is to change the definition of
our ordering (while still keeping it tight enough for the correctness guarantees that we need).
It looks almost like cheating, because we're going to redefine it precisely so that our early
stopping will work.

#def[
  Let $cal(S)_y$ be the set of all segments whose vertical range contains $y$. Say that $alpha weaklt_y beta$
  if $alpha weakprelt_y beta$ and there do not exist $alpha', beta' in cal(S)_y$ with $beta weakpreleq_y beta' stronglt alpha' weakpreleq_y alpha$.
]<def-weak-order>

In other words, we define $weaklt$ to be the same as $weakprelt$, but if there's ever a situation where the (not-yet-specified)
early stopping algorithm could miss a wrongly ordered pair $alpha weakprelt beta$ by witnessing a gap between $beta'$ and $alpha'$,
then we weaken the $weakprelt$ order and declare instead that $alpha weakeq beta$ instead of $alpha weaklt beta$.
To see how this solves our tricky example above, note that when instantiating @def-weak-order with $alpha = beta$, $beta = gamma$,
$beta' = alpha$, and $alpha' = beta$, we see that there *do* exist $alpha', beta' in cal(S)_y$ with the required properties,
and so in fact $beta$ and $gamma$ are not strictly ordered in the weak order $weaklt$ even though they were strictly ordered
in the weak pre-order $weakprelt$. In particular, the sweep-line $(gamma, alpha, beta)$ is legal with respect to the weak order $weaklt$.

You might object to @def-weak-order because of how hilariously inefficient it will be to implement. The
clever part, though, is that we won't need to implement it. Our algorithm will make its decisions based
only on the weak pre-order $weakprelt$ and the strong order $stronglt$. The inefficient weak order $weaklt$
will only be used in the correctness proof.
