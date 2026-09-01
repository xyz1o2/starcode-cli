<!--
name: 'Tool Description: ExitPlanMode'
description: Exit planning mode and execute
-->
Exit plan mode and begin executing the approved plan.

**Use for**: transitioning from planning to execution.
**NOT for**: when no plan exists, abandoning plan.

**Rules**:
- Only callable after `enter_plan_mode`
- Executes approved plan steps
- Reports progress as steps complete