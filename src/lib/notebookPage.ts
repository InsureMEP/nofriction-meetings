// The notebook page: typed wrappers over src-tauri/src/notebook_page.rs.
// Pure helpers are in notebookPageLogic.ts.

import { invoke } from "@tauri-apps/api/core";
import { asQuizQuestion, type NotebookOverview, type NotebookReview, type ReviewQuestion } from "./notebookPageLogic";

type RawReview = Omit<NotebookReview, "questions"> & {
    questions: (Omit<ReviewQuestion, "explanation"> & { explanation: string | null })[];
};

export const notebookApi = {
    overview: (notebook: string) => invoke<NotebookOverview>("get_notebook_overview", { notebook }),
    review: async (notebook: string): Promise<NotebookReview> => {
        const r = await invoke<RawReview>("get_notebook_review", { notebook });
        return { cards: r.cards, questions: r.questions.map(asQuizQuestion) };
    },
    /** Tick or untick an item (only a hash of its text is kept) */
    setDone: (meetingId: string, key: string, done: boolean) =>
        invoke<void>("set_notebook_item_done", { meetingId, key, done }),
};
