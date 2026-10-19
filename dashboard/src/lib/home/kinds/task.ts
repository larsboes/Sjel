import { projects, type Task } from "../../api";
import { link } from "../../nav";
import { RESCALE, type DecisionKind } from "../decisions";

/**
 * Band 620 — PRD §8.1, a task.
 *
 * A task outranks the mail it came from: someone already decided this is owed, where a
 * proposal is still awaiting that decision. Overdue and due-soon lift it; an undated task
 * sits at the base of the band.
 *
 * The vault's own 1/2/3 priority only breaks ties. A note the operator marked high is not
 * more urgent than one due tomorrow, so it is worth less than a single day of the
 * due-date slope.
 *
 * `capabilities/projects` is the one source (ISA.md F14). A row opens the dashboard's project page.
 *
 * Old expression: `620 + (due === null ? 0 : days < 0 ? 260 : max(0, 240 - days * 8))
 *                      + (3 - priority) * 3`, maximum 266.
 */
const task: DecisionKind<Task[], Task> = {
  key: "task",
  band: 620,
  label: "Tasks",
  capability: "projects",
  view: "TaskRow",

  load: (ctx) => projects.tasks("open", ctx.signal),
  // No gate: the source serves only open tasks.
  rows: (tasks) => tasks,
  id: (row) => row.id,
  title: (row) => row.title,
  urgency: (row, ctx) => {
    const days = row.due ? ctx.daysUntil(row.due) : null;
    const dated = days === null ? 0 : days < 0 ? 260 : Math.max(0, 240 - days * 8);
    return RESCALE(dated + (3 - row.priority) * 3, 266);
  },
  href: (row) => link(row.uri),

  whyHere: (row, ctx) => {
    if (!row.due) return "Still open, with no date on it.";
    const days = ctx.daysUntil(row.due);
    if (days < 0) return `Overdue by ${Math.abs(days)} day${Math.abs(days) === 1 ? "" : "s"}.`;
    if (days === 0) return "Due today.";
    return `Due in ${days} day${days === 1 ? "" : "s"}.`;
  },
  startOrDueAt: (row) => row.due,
  candidateStatus: () => "accepted",
  // CONTRACT: the projects server classifies every task it serves -- folder default, frontmatter
  // override (PRD Q9a), decided by `libs/content-item` and never here.
  dataClass: (row) => row.data_class ?? null,
  processingRoute: () => null,
};

export default task;
