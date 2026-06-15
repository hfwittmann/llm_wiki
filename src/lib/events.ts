//! Singleton EventSource wrapper with type-based subscription.
//!
//! The server emits SSE events on `/api/v1/events` (authenticated). Frontend
//! code subscribes by event type:
//!   const unsub = subscribe('ingest:progress', (payload) => { ... });
//! Multiple subscribers per event type are supported. The connection is
//! singleton (one EventSource per page) and reconnects automatically (the
//! browser handles that for us).

type Handler = (payload: unknown) => void;

class EventBus {
  private es: EventSource | null = null;
  private subscribers = new Map<string, Set<Handler>>();
  // Per-type listeners on the EventSource — we attach one DOM listener per
  // event type the first time someone subscribes to it.
  private attachedTypes = new Set<string>();

  constructor(private url: string) {}

  private ensureConnected(): EventSource {
    if (this.es && this.es.readyState !== EventSource.CLOSED) {
      return this.es;
    }
    const es = new EventSource(this.url, { withCredentials: true });
    this.es = es;
    // Re-attach all subscribed event types after a fresh connection.
    this.attachedTypes.clear();
    for (const eventType of this.subscribers.keys()) {
      this.attachEventListener(es, eventType);
    }
    return es;
  }

  private attachEventListener(es: EventSource, eventType: string) {
    if (this.attachedTypes.has(eventType)) return;
    this.attachedTypes.add(eventType);
    es.addEventListener(eventType, (e: MessageEvent) => {
      const payload = (() => {
        try {
          return JSON.parse(e.data);
        } catch {
          return e.data;
        }
      })();
      for (const h of this.subscribers.get(eventType) ?? []) {
        try {
          h(payload);
        } catch (err) {
          // Don't let one handler's throw kill others.
          // eslint-disable-next-line no-console
          console.error(`[events] handler for "${eventType}" threw:`, err);
        }
      }
    });
  }

  subscribe(eventType: string, handler: Handler): () => void {
    const es = this.ensureConnected();
    let set = this.subscribers.get(eventType);
    if (!set) {
      set = new Set();
      this.subscribers.set(eventType, set);
    }
    set.add(handler);
    this.attachEventListener(es, eventType);
    return () => {
      const cur = this.subscribers.get(eventType);
      if (cur) {
        cur.delete(handler);
        if (cur.size === 0) {
          this.subscribers.delete(eventType);
          // We leave the DOM listener attached; the EventSource will be
          // closed (and listeners discarded) when `close()` is called.
        }
      }
    };
  }

  close(): void {
    this.es?.close();
    this.es = null;
    this.subscribers.clear();
    this.attachedTypes.clear();
  }
}

const eventBus = new EventBus("/api/v1/events");

export function subscribe(
  eventType: string,
  handler: (payload: unknown) => void,
): () => void {
  return eventBus.subscribe(eventType, handler);
}

export function closeEventBus(): void {
  eventBus.close();
}

// Exported for testing only.
export { EventBus as _EventBusForTests };
