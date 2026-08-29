import { describe, expect, it } from "vitest";

import {
  applyBreakdownStepsIfCurrent,
  hasCandidateTitle,
  parseGoalLines,
  reconcileQuestionOneCandidates,
  type InterviewCandidate,
} from "./interview";

function q1(title: string, index: number): InterviewCandidate {
  return {
    key: `q1-${index}`,
    source: "q1",
    title,
    commitment_id: null,
    task_id: null,
    selected: index < 3,
    priority: "must",
    done_definition: index === 0 ? "A clear completion definition." : "",
    estimated_minutes: 60,
    steps: index === 0 ? ["Open the draft", "Send the result"] : [],
  };
}

describe("question-one candidate reconciliation", () => {
  it("replaces edited question-one goals instead of appending duplicates", () => {
    const initial = reconcileQuestionOneCandidates(
      [],
      "Finish sales playbook\nReview Baltimore pipeline\nSend Friday report\nPrepare team agenda",
    );

    const edited = reconcileQuestionOneCandidates(
      initial,
      "Finish and send sales playbook\nReview Baltimore pipeline\nSend Friday report\nPrepare team agenda",
    );

    expect(edited).toHaveLength(4);
    expect(edited.map((candidate) => candidate.title)).toEqual([
      "Finish and send sales playbook",
      "Review Baltimore pipeline",
      "Send Friday report",
      "Prepare team agenda",
    ]);
  });

  it("preserves metadata for an edited goal in the same slot", () => {
    const existing = { ...q1("Finish sales playbook", 0), commitment_id: 42 };
    const edited = reconcileQuestionOneCandidates(
      [existing],
      "Finish and send sales playbook",
    );

    expect(edited[0]).toMatchObject({
      commitment_id: 42,
      done_definition: "A clear completion definition.",
      steps: ["Open the draft", "Send the result"],
      selected: true,
    });
  });

  it("does not shift metadata onto a newly inserted goal", () => {
    const first = q1("Write proposal", 0);
    const second = {
      ...q1("Send invoices", 1),
      done_definition: "Every approved invoice is sent.",
      steps: ["Export approved invoices", "Send each invoice"],
    };

    const result = reconcileQuestionOneCandidates(
      [first, second],
      "Book travel\nWrite proposal\nSend invoices",
    );

    expect(result[0]).toMatchObject({
      title: "Book travel",
      done_definition: "",
      steps: [],
    });
    expect(result[1]).toMatchObject({
      title: "Write proposal",
      done_definition: first.done_definition,
      steps: first.steps,
    });
    expect(result[2]).toMatchObject({
      title: "Send invoices",
      done_definition: second.done_definition,
      steps: second.steps,
    });
  });

  it("does not select an inserted goal ahead of three preserved selections", () => {
    const result = reconcileQuestionOneCandidates(
      [q1("Write proposal", 0), q1("Send invoices", 1), q1("Review pipeline", 2)],
      "Book travel\nWrite proposal\nSend invoices\nReview pipeline",
    );

    expect(result[0]).toMatchObject({ title: "Book travel", selected: false });
    expect(result.filter((candidate) => candidate.selected)).toHaveLength(3);
  });

  it("does not shift metadata when insertion and removal keep the same count", () => {
    const first = q1("Write proposal", 0);
    const second = q1("Send invoices", 1);
    const third = q1("Review pipeline", 2);
    const result = reconcileQuestionOneCandidates(
      [first, second, third],
      "Book travel\nWrite proposal\nReview pipeline",
    );

    expect(result[0]).toMatchObject({ title: "Book travel", done_definition: "", steps: [] });
    expect(result[1]).toMatchObject({
      title: "Write proposal",
      done_definition: first.done_definition,
      steps: first.steps,
    });
    expect(result[2]).toMatchObject({
      title: "Review pipeline",
      done_definition: third.done_definition,
      steps: third.steps,
    });
  });

  it("keeps custom candidates while removing stale question-one candidates", () => {
    const custom: InterviewCandidate = {
      ...q1("Call the customer", 0),
      key: "custom-1",
      source: "custom",
    };
    const result = reconcileQuestionOneCandidates(
      [q1("Old outcome", 0), custom],
      "New outcome for today",
    );

    expect(result.map((candidate) => candidate.title)).toEqual([
      "New outcome for today",
      "Call the customer",
    ]);
  });

  it("deduplicates repeated lines and title checks ignore case and spacing", () => {
    expect(parseGoalLines("- Ship report\nship   report\n2) Review pipeline")).toEqual([
      "Ship report",
      "Review pipeline",
    ]);
    expect(hasCandidateTitle([q1("Ship   Report", 0)], " ship report ")).toBe(true);
    expect(parseGoalLines("2026 launch plan")).toEqual(["2026 launch plan"]);
  });
});

describe("goal breakdown request reconciliation", () => {
  it("applies steps when the requested goal and checklist are still current", () => {
    const candidate = q1("Write proposal", 0);
    const result = applyBreakdownStepsIfCurrent(
      [candidate],
      candidate,
      ["Open the brief", "Draft the proposal", "Send the proposal"],
    );

    expect(result[0]?.steps).toEqual([
      "Open the brief",
      "Draft the proposal",
      "Send the proposal",
    ]);
  });

  it("discards a result after the goal title or checklist changes", () => {
    const requested = q1("Write proposal", 0);
    const generated = ["Open the brief", "Draft the proposal", "Send the proposal"];
    const renamed = { ...requested, title: "Book customer travel" };
    const manuallyEdited = { ...requested, steps: ["Keep my manual step"] };

    expect(applyBreakdownStepsIfCurrent([renamed], requested, generated)[0]).toBe(renamed);
    expect(applyBreakdownStepsIfCurrent([manuallyEdited], requested, generated)[0]).toBe(
      manuallyEdited,
    );
  });
});
