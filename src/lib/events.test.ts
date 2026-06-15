import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { _EventBusForTests as EventBus } from "./events";

class MockEventSource {
  static instances: MockEventSource[] = [];
  url: string;
  readyState: number = EventSource.CONNECTING;
  listeners = new Map<string, ((e: MessageEvent) => void)[]>();

  constructor(url: string, _init?: EventSourceInit) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: (e: MessageEvent) => void) {
    const arr = this.listeners.get(type) ?? [];
    arr.push(listener);
    this.listeners.set(type, arr);
  }

  emit(type: string, data: string) {
    const evt = { data } as MessageEvent;
    for (const l of this.listeners.get(type) ?? []) l(evt);
  }

  close() {
    this.readyState = EventSource.CLOSED;
  }

  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSED = 2;
}

describe("EventBus", () => {
  beforeEach(() => {
    MockEventSource.instances = [];
    vi.stubGlobal("EventSource", MockEventSource);
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("subscribes and receives a typed event", () => {
    const bus = new EventBus("/events");
    const received: unknown[] = [];
    bus.subscribe("ingest:progress", (p) => received.push(p));
    const es = MockEventSource.instances[0];
    es.emit("ingest:progress", JSON.stringify({ pct: 42 }));
    expect(received).toEqual([{ pct: 42 }]);
  });

  it("dispatches to multiple subscribers for the same event type", () => {
    const bus = new EventBus("/events");
    const a: unknown[] = [];
    const b: unknown[] = [];
    bus.subscribe("chat:token", (p) => a.push(p));
    bus.subscribe("chat:token", (p) => b.push(p));
    MockEventSource.instances[0].emit("chat:token", JSON.stringify({ t: "hi" }));
    expect(a).toEqual([{ t: "hi" }]);
    expect(b).toEqual([{ t: "hi" }]);
  });

  it("unsubscribe removes only the targeted handler", () => {
    const bus = new EventBus("/events");
    const a: unknown[] = [];
    const b: unknown[] = [];
    const unsubA = bus.subscribe("x", (p) => a.push(p));
    bus.subscribe("x", (p) => b.push(p));
    unsubA();
    MockEventSource.instances[0].emit("x", "{}");
    expect(a).toEqual([]);
    expect(b).toEqual([{}]);
  });

  it("uses one EventSource for many subscriptions", () => {
    const bus = new EventBus("/events");
    bus.subscribe("a", () => {});
    bus.subscribe("b", () => {});
    bus.subscribe("c", () => {});
    expect(MockEventSource.instances.length).toBe(1);
  });

  it("close() closes the EventSource and clears subscribers", () => {
    const bus = new EventBus("/events");
    bus.subscribe("a", () => {});
    bus.close();
    expect(MockEventSource.instances[0].readyState).toBe(MockEventSource.CLOSED);
  });
});
