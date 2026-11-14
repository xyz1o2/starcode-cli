<!--
name: 'Tool Description: ExitPlanMode'
description: Exit planning mode after plan approval
-->
Exit plan mode after approval and seed tasks from the plan. The tool itself does not run the steps.

**Use for**: transitioning from planning to execution after the plan is approved.
**NOT for**: when no plan exists or the plan was rejected.

**Rules**:
- Call only after entering plan mode and receiving approval.
- Treat task creation as preparation, not proof that any plan step was executed.
- Continue the approved work through the normal tool flow when the runtime permits it; report only verified progress.