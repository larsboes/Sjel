export type InspectableType = "event" | "person" | "trip" | "transaction" | "layout" | "link";

export interface InspectableEvent {
  type: "event";
  id?: string;
  title: string;
  startsAt: string;
  endsAt?: string;
  allDay?: boolean;
  location?: string;
  commitment?: string;
  notes?: string;
  attendees?: string[];
  tripId?: string;
  onEdit?: () => void;
}

export interface InspectablePerson {
  type: "person";
  id: string;
  name: string;
  role?: string;
  relationship?: string;
  location?: string;
  status?: string;
  email?: string;
  notes?: string;
  trips?: { id: string; title: string; dates: string }[];
  events?: { id: string; title: string; date: string }[];
}

export interface InspectableTrip {
  type: "trip";
  id: string;
  title: string;
  destination: string;
  dates: string;
  companions?: string[];
  budget?: string;
  spent?: string;
  weather?: string;
  stages?: { type: "train" | "flight" | "stay" | "activity"; title: string; time: string; detail?: string }[];
}

export interface InspectableTransaction {
  type: "transaction";
  id: string;
  merchant: string;
  amount: string;
  date: string;
  category: string;
  trip?: string;
  notes?: string;
}

export interface InspectableLayout {
  type: "layout";
  id: string;
  name: string;
  pass: boolean;
  clearance?: string;
  itemsCount?: number;
  totalCost?: string;
}

/** A row another capability answered on `/api/links`, shown the same for every kind
 *  (libs/links/ISA.md D5). */
export interface InspectableLink {
  type: "link";
  id: string;
  kind: string;
  title: string;
  at?: string;
  meta?: string;
  /** The field that holds the reference, such as `trip_id`. */
  via: string;
}

export type InspectableItem =
  | InspectableEvent
  | InspectablePerson
  | InspectableTrip
  | InspectableTransaction
  | InspectableLayout
  | InspectableLink;

class InspectorStore {
  isOpen = $state(false);
  item = $state<InspectableItem | null>(null);
  /** Items passed through on the way here, so following a connection can be undone. */
  trail = $state<InspectableItem[]>([]);

  open(item: InspectableItem): void {
    this.item = item;
    this.trail = [];
    this.isOpen = true;
  }

  /** Follow a connection without leaving the panel. */
  follow(item: InspectableItem): void {
    if (this.item) this.trail.push(this.item);
    this.item = item;
  }

  back(): void {
    this.item = this.trail.pop() ?? this.item;
  }

  close(): void {
    this.isOpen = false;
    this.item = null;
    this.trail = [];
  }

  inspectEvent(event: Omit<InspectableEvent, "type">): void {
    this.open({ type: "event", ...event });
  }

  inspectPerson(person: Omit<InspectablePerson, "type">): void {
    this.open({ type: "person", ...person });
  }

  inspectTrip(trip: Omit<InspectableTrip, "type">): void {
    this.open({ type: "trip", ...trip });
  }

  inspectTransaction(tx: Omit<InspectableTransaction, "type">): void {
    this.open({ type: "transaction", ...tx });
  }

  inspectLayout(layout: Omit<InspectableLayout, "type">): void {
    this.open({ type: "layout", ...layout });
  }
}

export const inspectorStore = new InspectorStore();
