// The notebook page: typed wrappers over src-tauri/src/notebook_page.rs.
// Pure helpers are in notebookPageLogic.ts.

import { invoke } from "@tauri-apps/api/core";
import { asQuizQuestion, type NotebookOverview, type NotebookReview, type RecordingItem, type ReviewQuestion } from "./notebookPageLogic";

type RawReview = Omit<NotebookReview, "questions"> & {
    questions: (Omit<ReviewQuestion, "explanation"> & { explanation: string | null })[];
};

export const notebookApi = {
    overview: (notebook: string) => invoke<NotebookOverview>("get_notebook_overview", { notebook }),
    review: async (notebook: string): Promise<NotebookReview> => {
        const r = await invoke<RawReview>("get_notebook_review", { notebook });
        return { recordings: r.recordings ?? [], cards: r.cards, questions: r.questions.map(asQuizQuestion) };
    },
    /** One recording's items with their ticks, in its notes' order (Notes view) */
    recordingItems: (meetingId: string) => invoke<RecordingItem[]>("get_recording_items", { meetingId }),
    /** Tick or untick an item (only a hash of its text is kept) */
    setDone: (meetingId: string, key: string, done: boolean) =>
        invoke<void>("set_notebook_item_done", { meetingId, key, done }),
};
