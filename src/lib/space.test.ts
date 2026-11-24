// Run with `npm test` (node's test runner; Node strips the types).
import { test } from "node:test";
import assert from "node:assert/strict";
import { dueFor, formatBytes, keepLabel, parts, type RecordingSpace, type SpaceOverview } from "./space.ts";

test("sizes read the way Finder says them, and the way the backend does", () => {
    assert.equal(formatBytes(0), "0 KB");
    assert.equal(formatBytes(412_300), "412 KB");
    assert.equal(formatBytes(1_000_000), "1.0 MB");
    assert.equal(formatBytes(8_640_000), "8.6 MB");
    assert.equal(formatBytes(86_900_000), "87 MB");
    assert.equal(formatBytes(999_400_000), "999 MB");
    assert.equal(formatBytes(1_600_000_000), "1.6 GB");
    assert.equal(formatBytes(250_000_000_000), "250 GB");
    assert.equal(formatBytes(-5), "0 KB");
});

const rec = (id: string, daysAgo: number, bytes: number, over: Partial<RecordingSpace> = {}): RecordingSpace => ({
    meeting_id: id,
    title: id,
    started_at: new Date(Date.UTC(2026, 10, 24) - daysAgo * 86_400_000).toISOString(),
    screens_bytes: bytes,
    screens_count: bytes ? 3 : 0,
    video_bytes: 0,
    total_bytes: bytes,
    saves_bytes: 0,
    recording: false,
    ...over,
});

test("keeping screens for a time names only older recordings that still hold something", () => {
    const now = new Date(Date.UTC(2026, 10, 24));
    const list = [rec("old", 120, 5e6), rec("empty", 200, 0), rec("new", 10, 7e6), rec("live", 400, 9e6, { recording: true })];
    assert.deepEqual(dueFor(list, 90, now).map((r) => r.meeting_id), ["old"]);
    assert.deepEqual(dueFor(list, 30, now).map((r) => r.meeting_id), ["old"]);
    assert.deepEqual(dueFor(list, 0, now), [], "forever keeps everything");
    assert.equal(keepLabel(0), "Forever");
    assert.equal(keepLabel(365), "1 year");
    assert.equal(keepLabel(90), "90 days");
});

test("the bar shows only the parts that hold something", () => {
    const o: SpaceOverview = {
        total_bytes: 100,
        free_bytes: null,
        screens_bytes: 60,
        screens_count: 4,
        video_bytes: 0,
        models_bytes: 30,
        database_bytes: 10,
        leftover_bytes: 0,
        other_bytes: 0,
        shrink: { pictures: 0, saves_bytes: 0 },
        recordings: [],
        screen_size: "smaller",
        keep_screens_days: 0,
    };
    assert.deepEqual(parts(o).map((p) => p.id), ["screens", "models", "library"]);
    assert.equal(parts(o).reduce((n, p) => n + p.bytes, 0), o.total_bytes);
});
