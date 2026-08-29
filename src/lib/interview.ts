import type { Priority } from "@/lib/ipc";

export type CandidateSource = "q1" | "custom" | "backlog";

export interface InterviewCandidate {
  key: string;
  source: CandidateSource;
  title: string;
  task_id: number | null;
  selected: boolean;
  priority: Priority;
  done_definition: string;
  estimated_minutes: number | null;
  steps: string[];
}

export function normalizedCandidateTitle(title: string): string {
  return title.trim().replace(/\s+/g, " ").toLowerCase();
}

export function parseGoalLines(answer: string): string[] {
  const seen = new Set<string>();
  return answer
    .split(/\n|;/)
    .map((line) =>
      line
        .replace(/^(?:[-*]\s+|\d+[.)]\s*)/, "")
        .trim()
        .replace(/\s+/g, " "),
    )
    .filter((line) => {
      if (line.length < 5) return false;
      const normalized = normalizedCandidateTitle(line);
      if (seen.has(normalized)) return false;
      seen.add(normalized);
      return true;
    });
}

/**
 * Question 1 owns one replaceable group of candidates. Re-entering question 1
 * must reconcile that group, not append another copy. Custom and backlog rows
 * remain intact, while matching/edited Q1 rows retain the user's selections,
 * DONE definitions, estimates, and generated action steps.
 */
export function reconcileQuestionOneCandidates(
  previous: InterviewCandidate[],
  answer: string,
): InterviewCandidate[] {
  const priorQuestionOne = previous.filter((candidate) => candidate.source === "q1");
  const questionOneTitles = parseGoalLines(answer);
  const retained: InterviewCandidate[] = [];
  const seen = new Set<string>();

  for (const candidate of previous) {
    if (candidate.source === "q1") continue;
    const normalized = normalizedCandidateTitle(candidate.title);
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    retained.push(candidate);
  }

  // Reserve every exact match before considering same-slot edits. Without this
  // first pass, inserting a goal before an existing one can give the inserted
  // row the existing goal's selection, DONE definition, and generated steps.
  const priorByNewIndex = new Map<number, number>();
  const reservedPriorIndexes = new Set<number>();
  for (const [newIndex, title] of questionOneTitles.entries()) {
    const normalized = normalizedCandidateTitle(title);
    const priorIndex = priorQuestionOne.findIndex(
      (candidate, candidateIndex) =>
        !reservedPriorIndexes.has(candidateIndex) &&
        normalizedCandidateTitle(candidate.title) === normalized,
    );
    if (priorIndex >= 0) {
      priorByNewIndex.set(newIndex, priorIndex);
      reservedPriorIndexes.add(priorIndex);
    }
  }

  const sameNumberOfGoals = priorQuestionOne.length === questionOneTitles.length;
  const exactMatchesStayInPlace = [...priorByNewIndex].every(
    ([newIndex, priorIndex]) => newIndex === priorIndex,
  );
  if (sameNumberOfGoals && exactMatchesStayInPlace) {
    for (const index of questionOneTitles.keys()) {
      if (
        !priorByNewIndex.has(index) &&
        priorQuestionOne[index] &&
        !reservedPriorIndexes.has(index)
      ) {
        priorByNewIndex.set(index, index);
        reservedPriorIndexes.add(index);
      }
    }
  }

  // Count preserved selections before default-selecting new goals, regardless
  // of where an inserted row appears in the new list.
  let selectedCount = retained.filter((candidate) => candidate.selected).length;
  for (const [index, title] of questionOneTitles.entries()) {
    if (seen.has(normalizedCandidateTitle(title))) continue;
    const priorIndex = priorByNewIndex.get(index);
    if (priorIndex !== undefined && priorQuestionOne[priorIndex]?.selected) {
      selectedCount += 1;
    }
  }
  const fromQuestionOne: InterviewCandidate[] = [];

  for (const [index, title] of questionOneTitles.entries()) {
    const normalized = normalizedCandidateTitle(title);
    // A manually added or backlog candidate is already the same outcome.
    if (seen.has(normalized)) continue;

    // Positional matches were only added above when no row was added or
    // removed, exact matches did not move, and the old slot was unreserved.
    const priorIndex = priorByNewIndex.get(index) ?? -1;
    const prior = priorIndex >= 0 ? priorQuestionOne[priorIndex] : undefined;

    const selected = prior?.selected ?? selectedCount < 3;
    if (!prior && selected) selectedCount += 1;
    fromQuestionOne.push({
      key: `q1-${index}`,
      source: "q1",
      title,
      task_id: null,
      selected,
      priority: prior?.priority ?? "must",
      done_definition: prior?.done_definition ?? "",
      estimated_minutes: prior?.estimated_minutes ?? 60,
      steps: prior?.steps ?? [],
    });
    seen.add(normalized);
  }

  return [...fromQuestionOne, ...retained];
}

export function hasCandidateTitle(candidates: InterviewCandidate[], title: string): boolean {
  const normalized = normalizedCandidateTitle(title);
  return candidates.some(
    (candidate) => normalizedCandidateTitle(candidate.title) === normalized,
  );
}

/**
 * Applies an asynchronous AI result only while its goal and checklist are the
 * same objects the request was created from. A title reconciliation or manual
 * checklist edit makes the result stale and leaves current state untouched.
 */
export function applyBreakdownStepsIfCurrent(
  candidates: InterviewCandidate[],
  request: Pick<InterviewCandidate, "key" | "title" | "steps">,
  generatedSteps: string[],
): InterviewCandidate[] {
  return candidates.map((candidate) => {
    if (
      candidate.key !== request.key ||
      normalizedCandidateTitle(candidate.title) !== normalizedCandidateTitle(request.title) ||
      candidate.steps !== request.steps
    ) {
      return candidate;
    }
    return { ...candidate, steps: generatedSteps };
  });
}
