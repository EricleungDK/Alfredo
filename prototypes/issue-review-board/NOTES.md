# Issue Review Board Prototype Notes

Status: throwaway prototype.

Question being answered:

Should the Issue Slice review and model-assignment step be a generated HTML artifact, a permanent mission-control UI screen, or a hybrid where structured issue data is rendered in-app and can export/share an HTML summary?

How to run:

```bash
python3 -m http.server 8787 --directory prototypes/issue-review-board
```

Then open:

- `http://localhost:8787/?variant=report`
- `http://localhost:8787/?variant=mission`
- `http://localhost:8787/?variant=hybrid`

Variants:

- `report`: generated HTML review packet. Strong for async reading and archival. Weak for steering because edits feel like annotations on a document.
- `mission`: permanent mission-control board. Strong for triage, dependency reading, fast decisions, and staying in the same operational surface when switching from review queue to execution board.
- `hybrid`: app-native review board with exportable summary. Strong if the board is the source of truth and the HTML artifact is only a share/archive output.

Variant B flow note:

- `View execution board` stays inside `?variant=mission` and toggles the center/right mission content into execution mode.
- Variant C remains a separate comparison for the hybrid concept. Variant B no longer routes to Variant C.

Prototype verdict placeholder:

- Winning direction:
- Borrow from other variants:
- What must remain editable at review time:
- What can be hidden behind drill-down:
- Desired model-assignment control before launch:

Cleanup rule:

Delete this folder or fold the selected interaction pattern into the real app once the question is answered.
