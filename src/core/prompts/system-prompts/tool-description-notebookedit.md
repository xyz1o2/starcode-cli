<!--
name: 'Tool Description: NotebookEdit'
description: Edit Jupyter notebook cells
-->
Edit cells in Jupyter notebook (.ipynb) files.

**Use for**: modifying notebook cells, adding/removing cells.
**NOT for**: regular Python files (use `replace`), reading notebooks (use `notebook_read`).

**Rules**:
- Specify cell index for modification
- Supports cell type changes (code/markdown)
- Preserve notebook JSON structure