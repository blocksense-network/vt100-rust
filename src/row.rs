use crate::term::BufWrite as _;
use unicode_width::UnicodeWidthStr as _;

/// What a full redraw or a diff leaves to print after all the rows
/// (`Grid::write_deferred_runs`), once moving the cursor no longer matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deferred {
    /// The cells before the cell at this position, the last one of its row:
    /// a run of emoji cells kept apart (`Row::write_cell_apart`).
    Run(crate::grid::Pos),
    /// The cell at this position, the last one of a wrapped row, that holds
    /// the cursor on it (`Row::holds_cursor`), printed as a space so the row
    /// could wrap after it (`Row::write_held_cell_waiting`).
    Cell(crate::grid::Pos),
}

#[derive(Clone, Debug)]
pub struct Row {
    cells: Vec<crate::Cell>,
    wrapped: bool,
}

impl Row {
    pub fn new(cols: u16) -> Self {
        Self {
            cells: vec![crate::Cell::new(); usize::from(cols)],
            wrapped: false,
        }
    }

    fn cols(&self) -> u16 {
        self.cells
            .len()
            .try_into()
            // we limit the number of cols to a u16 (see Size)
            .unwrap()
    }

    pub fn clear(&mut self, attrs: crate::attrs::Attrs) {
        for cell in &mut self.cells {
            cell.clear(attrs);
        }
        self.wrapped = false;
    }

    fn cells(&self) -> impl Iterator<Item = &crate::Cell> {
        self.cells.iter()
    }

    pub fn get(&self, col: u16) -> Option<&crate::Cell> {
        self.cells.get(usize::from(col))
    }

    pub fn get_mut(&mut self, col: u16) -> Option<&mut crate::Cell> {
        self.cells.get_mut(usize::from(col))
    }

    pub fn insert(&mut self, i: u16, cell: crate::Cell) {
        self.cells.insert(usize::from(i), cell);
        self.wrapped = false;
    }

    pub fn remove(&mut self, i: u16) {
        self.clear_wide(i);
        self.cells.remove(usize::from(i));
        self.wrapped = false;
    }

    pub fn erase(&mut self, i: u16, attrs: crate::attrs::Attrs) {
        let wide = self.cells[usize::from(i)].is_wide();
        self.clear_wide(i);
        self.cells[usize::from(i)].clear(attrs);
        if i == self.cols() - if wide { 2 } else { 1 } {
            self.wrapped = false;
        }
    }

    pub fn truncate(&mut self, len: u16) {
        self.cells.truncate(usize::from(len));
        self.wrapped = false;
        let last_cell = &mut self.cells[usize::from(len) - 1];
        if last_cell.is_wide() {
            last_cell.clear(*last_cell.attrs());
        }
    }

    pub fn resize(&mut self, len: u16, cell: crate::Cell) {
        self.cells.resize(usize::from(len), cell);
        self.wrapped = false;
    }

    pub fn wrap(&mut self, wrap: bool) {
        self.wrapped = wrap;
    }

    pub fn wrapped(&self) -> bool {
        self.wrapped
    }

    pub fn clear_wide(&mut self, col: u16) {
        let cell = &self.cells[usize::from(col)];
        let other = if cell.is_wide() {
            &mut self.cells[usize::from(col + 1)]
        } else if cell.is_wide_continuation() {
            &mut self.cells[usize::from(col - 1)]
        } else {
            return;
        };
        other.clear(*other.attrs());
    }

    pub fn write_contents(
        &self,
        contents: &mut String,
        start: u16,
        width: u16,
        wrapping: bool,
    ) {
        let mut prev_was_wide = false;

        let mut prev_col = start;
        for (col, cell) in self
            .cells()
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            if cell.has_contents() {
                for _ in 0..(col - prev_col) {
                    contents.push(' ');
                }
                prev_col += col - prev_col;

                contents.push_str(cell.contents());
                prev_col += if cell.is_wide() { 2 } else { 1 };
            }
        }
        if prev_col == start && wrapping {
            contents.push('\n');
        }
    }

    // The column where the cell before `col` on this row starts (the first
    // half when it is a wide character): the cell a character printed at
    // `col` follows. None at `start`, the first column written.
    fn left_cell_start(&self, col: u16, start: u16) -> Option<u16> {
        if col <= start {
            return None;
        }
        let left = col - 1;
        if left > start
            && self.cells[usize::from(left)].is_wide_continuation()
        {
            return Some(left - 1);
        }
        Some(left)
    }

    // Whether the cell starting at `col`, printed with the cursor right after
    // the cell before it, would join that cell's emoji cluster. The parser
    // joins on the cursor's position alone, as tmux does, so a program can
    // leave two such cells apart (by writing the second one first, or moving
    // the cursor between them), and printing them in order would rebuild one
    // cell where this row has two.
    pub fn joins_left(&self, col: u16, start: u16) -> bool {
        self.left_cell_start(col, start).is_some_and(|left| {
            self.cells[usize::from(col)]
                .would_join(&self.cells[usize::from(left)])
        })
    }

    // Whether this row's first cell would join the last cell of `above`, the
    // row this one continues, when printed with the cursor waiting to wrap
    // after that cell: the parser then takes the cell before the cursor on
    // the row above. A cursor moved to the first column does not join
    // across the row, so a program can leave the two apart.
    fn first_cell_joins(&self, above: Option<&Self>) -> bool {
        let Some(above) = above else {
            return false;
        };
        let last = above.cols() - 1;
        let last = if last > 0
            && above.cells[usize::from(last)].is_wide_continuation()
        {
            last - 1
        } else {
            last
        };
        self.cells[0].would_join(&above.cells[usize::from(last)])
    }

    // Whether the cell at `col` keeps the cursor on it when printed: a cell
    // in the last column whose contents are two columns wide (a cluster
    // completed there, or a character with VS16) stays one column wide, and
    // the parser ends the pending wrap with the cursor on it, as tmux 3.6a
    // does (Screen::widen_cluster). Printing it cannot leave the cursor
    // waiting to wrap after it, which a wrapped row's last cell has to.
    pub fn holds_cursor(&self, col: u16) -> bool {
        let cell = &self.cells[usize::from(col)];
        col == self.cols() - 1
            && cell.has_contents()
            && !cell.is_wide()
            && cell.contents().width() == 2
    }

    // The column the cursor is in after the cell starting at `col` was
    // printed from there: past it, or on it when it holds the cursor.
    fn col_after(&self, col: u16) -> u16 {
        if self.holds_cursor(col) {
            col
        } else {
            col + if self.cells[usize::from(col)].is_wide() {
                2
            } else {
                1
            }
        }
    }

    // Prints the last cell of this wrapped row, which holds the cursor on it
    // (holds_cursor), with the cursor at it and its attributes current, so
    // that the cursor waits to wrap after it: a space is printed there
    // first, which leaves the cursor waiting, for the next row's first
    // character to wrap the row. With `deferred` given (the whole grid is
    // being printed), the cell is pushed to it and printed again after the
    // rows below (write_deferred_cell). Otherwise it is printed again now,
    // between a saved and a restored cursor, which this parser restores
    // past the end of the row; tmux 3.6a restores it into the last column
    // (see write_cell_apart).
    //
    // With `run`, the cell would join the cell before it, and
    // write_cell_apart has erased the run of such cells that ends at it:
    // the run is printed after the cell (deferred too, after it), so that
    // the cell is printed after an erased cell, which it cannot join.
    fn write_held_cell_waiting(
        &self,
        contents: &mut Vec<u8>,
        row: u16,
        col: u16,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
        deferred: Option<&mut Vec<Deferred>>,
        run: bool,
    ) {
        let pos = crate::grid::Pos { row, col };
        contents.push(b' ');
        *prev_pos = crate::grid::Pos {
            row,
            col: self.cols(),
        };
        if let Some(deferred) = deferred {
            // contents_diff can print the row's last cell twice (again to
            // wrap the row); the cell must be printed before its run, which
            // it would join otherwise
            let mut items = vec![Deferred::Cell(pos)];
            if run {
                items.push(Deferred::Run(pos));
            }
            for item in items {
                if !deferred.contains(&item) {
                    deferred.push(item);
                }
            }
            return;
        }
        let saved = (*prev_pos, *prev_attrs);
        crate::term::SaveCursor.write_buf(contents);
        self.write_cell(contents, row, col, prev_pos, prev_attrs);
        if run {
            self.write_run_before(
                contents, row, col, 0, prev_pos, prev_attrs,
            );
        }
        crate::term::RestoreCursor.write_buf(contents);
        (*prev_pos, *prev_attrs) = saved;
    }

    /// Prints the last cell of a wrapped row that `write_held_cell_waiting`
    /// left a space and deferred, now that the rows below have been printed.
    /// The cursor is left on it.
    pub fn write_deferred_cell(
        &self,
        contents: &mut Vec<u8>,
        pos: crate::grid::Pos,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
    ) {
        // The move is absolute (the cursor is not on this row's last
        // column), so the cell is printed with the cursor at it, as the
        // program left it. It does not join the cell before it, or that
        // cell was erased with its run (write_cell_apart), which is printed
        // after this one.
        crate::term::MoveTo::new(pos).write_buf(contents);
        *prev_pos = pos;
        self.write_cell(contents, pos.row, pos.col, prev_pos, prev_attrs);
    }

    // Prints the cell starting at `col` with the cursor at `prev_pos`, and
    // moves `prev_pos` past it (or leaves it on a cell that holds the
    // cursor).
    fn write_cell(
        &self,
        contents: &mut Vec<u8>,
        row: u16,
        col: u16,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
    ) {
        let pos = crate::grid::Pos { row, col };
        let cell = &self.cells[usize::from(col)];
        crate::term::MoveFromToInRow::new(*prev_pos, pos).write_buf(contents);
        if prev_attrs != cell.attrs() {
            cell.attrs().write_escape_code_diff(contents, prev_attrs);
            *prev_attrs = *cell.attrs();
        }
        contents.extend(cell.contents().as_bytes());
        *prev_pos = crate::grid::Pos {
            row,
            col: self.col_after(col),
        };
    }

    /// Prints the cell starting at `col` with the cursor at `prev_pos`, kept
    /// apart from the cell before it when it would join that cell, and moves
    /// `prev_pos` and `prev_attrs` to where the cursor and the attributes are
    /// left. A cell that ends in the last column leaves the cursor waiting to
    /// wrap after it.
    pub fn redraw_cell(
        &self,
        contents: &mut Vec<u8>,
        row: u16,
        col: u16,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
    ) {
        if self.joins_left(col, 0) {
            self.write_cell_apart(
                contents, row, col, 0, true, prev_pos, prev_attrs, None,
            );
        } else if self.holds_cursor(col) {
            let pos = crate::grid::Pos { row, col };
            let cell = &self.cells[usize::from(col)];
            crate::term::MoveFromToInRow::new(*prev_pos, pos).write_buf(contents);
            if prev_attrs != cell.attrs() {
                cell.attrs().write_escape_code_diff(contents, prev_attrs);
                *prev_attrs = *cell.attrs();
            }
            self.write_held_cell_waiting(
                contents, row, col, prev_pos, prev_attrs, None, false,
            );
        } else {
            self.write_cell(contents, row, col, prev_pos, prev_attrs);
        }
    }

    // Prints the cell starting at `col`, which would join the cell before it
    // (joins_left), so that the two stay apart, as the program left them.
    // The cell before it may join the one before that in turn, so this takes
    // the whole run of such cells that ends at `col`: it erases the run's
    // other cells, prints the cell at `col` (after an erased cell, which it
    // cannot join), then prints the others again from right to left, each
    // after an erased cell, and the run's first after a cell it does not
    // join. Every cell then has its contents and attributes; the erase used
    // the current attributes, but each erased cell is printed again.
    //
    // A cell that ends in the last column leaves the cursor waiting to wrap,
    // which the next row's first character (it wraps the row) or the final
    // cursor position may rely on, and only printing a character there
    // leaves it so. With `deferred` given (the whole grid is being printed),
    // the run's other cells are left erased and the cell's position is
    // pushed to it: the caller prints them after the rows below
    // (write_deferred_run), where moving the cursor no longer matters, and
    // reaches a final cursor waiting to wrap there on its own
    // (Grid::write_cursor_position_formatted). Otherwise the cursor is
    // saved after the cell and restored after the others, which also
    // restores the attributes the cell was printed with. DECSC and DECRC
    // keep the column past the end in this parser (as
    // Grid::write_cursor_position_formatted relies on too), but tmux 3.6a
    // restores the cursor into the last column, so the next character would
    // be printed over the cell instead of wrapping; deferring avoids that.
    // Otherwise the cursor is left after the run's first cell.
    //
    // With `wait` (its row is wrapped, or the final cursor waits after it), a
    // cell that holds the cursor on it (holds_cursor) is printed as a space
    // instead, and then again before its run (write_held_cell_waiting).
    fn write_cell_apart(
        &self,
        contents: &mut Vec<u8>,
        row: u16,
        col: u16,
        start: u16,
        wait: bool,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
        deferred: Option<&mut Vec<Deferred>>,
    ) {
        let mut first = col;
        while self.joins_left(first, start) {
            // joins_left found the cell before `first`
            first = self.left_cell_start(first, start).unwrap();
        }
        let first_pos = crate::grid::Pos { row, col: first };
        crate::term::MoveFromToInRow::new(*prev_pos, first_pos)
            .write_buf(contents);
        crate::term::EraseChar::new(col - first).write_buf(contents);
        *prev_pos = first_pos;

        if wait && self.holds_cursor(col) {
            let pos = crate::grid::Pos { row, col };
            let cell = &self.cells[usize::from(col)];
            crate::term::MoveFromToInRow::new(*prev_pos, pos).write_buf(contents);
            *prev_pos = pos;
            if prev_attrs != cell.attrs() {
                cell.attrs().write_escape_code_diff(contents, prev_attrs);
                *prev_attrs = *cell.attrs();
            }
            self.write_held_cell_waiting(
                contents, row, col, prev_pos, prev_attrs, deferred, true,
            );
            return;
        }

        self.write_cell(contents, row, col, prev_pos, prev_attrs);
        let waits_to_wrap = prev_pos.col >= self.cols();
        if waits_to_wrap {
            if let Some(deferred) = deferred {
                // contents_diff can print the row's last cell twice (again
                // to wrap the row); its run must be printed once, after an
                // erased cell each, or the second pass would join it
                let pos = Deferred::Run(crate::grid::Pos { row, col });
                if !deferred.contains(&pos) {
                    deferred.push(pos);
                }
                return;
            }
        }
        let saved = (*prev_pos, *prev_attrs);
        if waits_to_wrap {
            crate::term::SaveCursor.write_buf(contents);
        }
        self.write_run_before(
            contents, row, col, start, prev_pos, prev_attrs,
        );
        if waits_to_wrap {
            crate::term::RestoreCursor.write_buf(contents);
            (*prev_pos, *prev_attrs) = saved;
        }
    }

    // Prints the cells of the run that ends at the cell starting at `col`
    // (see write_cell_apart), other than that cell, from right to left: each
    // after an erased cell, and the run's first after a cell it does not
    // join. Leaves the cursor after the run's first cell.
    fn write_run_before(
        &self,
        contents: &mut Vec<u8>,
        row: u16,
        col: u16,
        start: u16,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
    ) {
        let mut next = col;
        while self.joins_left(next, start) {
            // joins_left found the cell before `next`
            next = self.left_cell_start(next, start).unwrap();
            self.write_cell(contents, row, next, prev_pos, prev_attrs);
        }
    }

    /// Prints the cells `write_cell_apart` left erased before the cell at
    /// `pos` (the last one of its row) and deferred, now that the rows
    /// below have been printed.
    pub fn write_deferred_run(
        &self,
        contents: &mut Vec<u8>,
        pos: crate::grid::Pos,
        prev_pos: &mut crate::grid::Pos,
        prev_attrs: &mut crate::attrs::Attrs,
    ) {
        self.write_run_before(
            contents, pos.row, pos.col, 0, prev_pos, prev_attrs,
        );
    }

    pub fn write_contents_formatted(
        &self,
        contents: &mut Vec<u8>,
        start: u16,
        width: u16,
        row: u16,
        wrapping: bool,
        above: Option<&Self>,
        mut deferred: Option<&mut Vec<Deferred>>,
        prev_pos: Option<crate::grid::Pos>,
        prev_attrs: Option<crate::attrs::Attrs>,
    ) -> (crate::grid::Pos, crate::attrs::Attrs) {
        let mut prev_was_wide = false;
        let default_cell = crate::Cell::new();

        let mut prev_pos = prev_pos.unwrap_or_else(|| {
            if wrapping {
                crate::grid::Pos {
                    row: row - 1,
                    col: self.cols(),
                }
            } else {
                crate::grid::Pos { row, col: start }
            }
        });
        let mut prev_attrs = prev_attrs.unwrap_or_default();

        let first_cell = &self.cells[usize::from(start)];
        if wrapping && first_cell == &default_cell {
            let default_attrs = default_cell.attrs();
            if &prev_attrs != default_attrs {
                default_attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *default_attrs;
            }
            contents.push(b' ');
            crate::term::Backspace.write_buf(contents);
            crate::term::EraseChar::new(1).write_buf(contents);
            prev_pos = crate::grid::Pos { row, col: 0 };
        }

        let mut erase: Option<(u16, &crate::attrs::Attrs)> = None;
        for (col, cell) in self
            .cells()
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            let pos = crate::grid::Pos { row, col };

            if let Some((prev_col, attrs)) = erase {
                if cell.has_contents() || cell.attrs() != attrs {
                    let new_pos = crate::grid::Pos { row, col: prev_col };
                    if wrapping
                        && prev_pos.row + 1 == new_pos.row
                        && prev_pos.col >= self.cols()
                    {
                        if new_pos.col > 0 {
                            contents.extend(
                                " ".repeat(usize::from(new_pos.col))
                                    .as_bytes(),
                            );
                        } else {
                            contents.extend(b" ");
                            crate::term::Backspace.write_buf(contents);
                        }
                    } else {
                        crate::term::MoveFromTo::new(prev_pos, new_pos)
                            .write_buf(contents);
                    }
                    prev_pos = new_pos;
                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }
                    crate::term::EraseChar::new(pos.col - prev_col)
                        .write_buf(contents);
                    erase = None;
                }
            }

            if cell != &default_cell {
                let attrs = cell.attrs();
                if cell.has_contents() && self.joins_left(col, start) {
                    self.write_cell_apart(
                        contents,
                        row,
                        col,
                        start,
                        self.wrapped,
                        &mut prev_pos,
                        &mut prev_attrs,
                        deferred.as_deref_mut(),
                    );
                } else if cell.has_contents() {
                    if pos != prev_pos {
                        if !wrapping
                            || prev_pos.row + 1 != pos.row
                            || prev_pos.col
                                < self.cols() - u16::from(cell.is_wide())
                            || pos.col != 0
                        {
                            crate::term::MoveFromTo::new(prev_pos, pos)
                                .write_buf(contents);
                        } else if self.first_cell_joins(above) {
                            write_wrap_apart(contents, prev_pos, self.cols());
                        }
                        prev_pos = pos;
                    }

                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }

                    if self.wrapped && self.holds_cursor(col) {
                        self.write_held_cell_waiting(
                            contents,
                            row,
                            col,
                            &mut prev_pos,
                            &mut prev_attrs,
                            deferred.as_deref_mut(),
                            false,
                        );
                    } else {
                        prev_pos.col = self.col_after(col);
                        contents.extend(cell.contents().as_bytes());
                    }
                } else if erase.is_none() {
                    erase = Some((pos.col, attrs));
                }
            }
        }
        if let Some((prev_col, attrs)) = erase {
            let new_pos = crate::grid::Pos { row, col: prev_col };
            if wrapping
                && prev_pos.row + 1 == new_pos.row
                && prev_pos.col >= self.cols()
            {
                if new_pos.col > 0 {
                    contents.extend(
                        " ".repeat(usize::from(new_pos.col)).as_bytes(),
                    );
                } else {
                    contents.extend(b" ");
                    crate::term::Backspace.write_buf(contents);
                }
            } else {
                crate::term::MoveFromTo::new(prev_pos, new_pos)
                    .write_buf(contents);
            }
            prev_pos = new_pos;
            if &prev_attrs != attrs {
                attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *attrs;
            }
            crate::term::ClearRowForward.write_buf(contents);
        }

        (prev_pos, prev_attrs)
    }

    // while it's true that most of the logic in this is identical to
    // write_contents_formatted, i can't figure out how to break out the
    // common parts without making things noticeably slower.
    pub fn write_contents_diff(
        &self,
        contents: &mut Vec<u8>,
        prev: &Self,
        start: u16,
        width: u16,
        row: u16,
        wrapping: bool,
        prev_wrapping: bool,
        above: Option<&Self>,
        mut deferred: Option<&mut Vec<Deferred>>,
        mut prev_pos: crate::grid::Pos,
        mut prev_attrs: crate::attrs::Attrs,
    ) -> (crate::grid::Pos, crate::attrs::Attrs) {
        let mut prev_was_wide = false;

        let first_cell = &self.cells[usize::from(start)];
        let prev_first_cell = &prev.cells[usize::from(start)];
        if wrapping
            && !prev_wrapping
            && first_cell == prev_first_cell
            && prev_pos.row + 1 == row
            && prev_pos.col
                >= self.cols() - u16::from(prev_first_cell.is_wide())
        {
            let first_cell_attrs = first_cell.attrs();
            if &prev_attrs != first_cell_attrs {
                first_cell_attrs
                    .write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *first_cell_attrs;
            }
            let mut cell_contents = prev_first_cell.contents();
            let need_erase = if cell_contents.is_empty() {
                cell_contents = " ";
                true
            } else {
                false
            };
            if self.first_cell_joins(above) {
                write_wrap_apart(contents, prev_pos, self.cols());
            }
            contents.extend(cell_contents.as_bytes());
            crate::term::Backspace.write_buf(contents);
            if prev_first_cell.is_wide() {
                crate::term::Backspace.write_buf(contents);
            }
            if need_erase {
                crate::term::EraseChar::new(1).write_buf(contents);
            }
            prev_pos = crate::grid::Pos { row, col: 0 };
        }

        let mut erase: Option<(u16, &crate::attrs::Attrs)> = None;
        for (col, (cell, prev_cell)) in self
            .cells()
            .zip(prev.cells())
            .enumerate()
            .skip(usize::from(start))
            .take(usize::from(width))
        {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            prev_was_wide = cell.is_wide();

            // we limit the number of cols to a u16 (see Size)
            let col: u16 = col.try_into().unwrap();
            let pos = crate::grid::Pos { row, col };

            if let Some((prev_col, attrs)) = erase {
                if cell.has_contents() || cell.attrs() != attrs {
                    let new_pos = crate::grid::Pos { row, col: prev_col };
                    if wrapping
                        && prev_pos.row + 1 == new_pos.row
                        && prev_pos.col >= self.cols()
                    {
                        if new_pos.col > 0 {
                            contents.extend(
                                " ".repeat(usize::from(new_pos.col))
                                    .as_bytes(),
                            );
                        } else {
                            contents.extend(b" ");
                            crate::term::Backspace.write_buf(contents);
                        }
                    } else {
                        crate::term::MoveFromTo::new(prev_pos, new_pos)
                            .write_buf(contents);
                    }
                    prev_pos = new_pos;
                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }
                    crate::term::EraseChar::new(pos.col - prev_col)
                        .write_buf(contents);
                    erase = None;
                }
            }

            if cell != prev_cell {
                let attrs = cell.attrs();
                if cell.has_contents() && self.joins_left(col, start) {
                    self.write_cell_apart(
                        contents,
                        row,
                        col,
                        start,
                        self.wrapped,
                        &mut prev_pos,
                        &mut prev_attrs,
                        deferred.as_deref_mut(),
                    );
                } else if cell.has_contents() {
                    if pos != prev_pos {
                        if !wrapping
                            || prev_pos.row + 1 != pos.row
                            || prev_pos.col
                                < self.cols() - u16::from(cell.is_wide())
                            || pos.col != 0
                        {
                            crate::term::MoveFromTo::new(prev_pos, pos)
                                .write_buf(contents);
                        } else if self.first_cell_joins(above) {
                            write_wrap_apart(contents, prev_pos, self.cols());
                        }
                        prev_pos = pos;
                    }

                    if &prev_attrs != attrs {
                        attrs.write_escape_code_diff(contents, &prev_attrs);
                        prev_attrs = *attrs;
                    }

                    if self.wrapped && self.holds_cursor(col) {
                        self.write_held_cell_waiting(
                            contents,
                            row,
                            col,
                            &mut prev_pos,
                            &mut prev_attrs,
                            deferred.as_deref_mut(),
                            false,
                        );
                    } else {
                        prev_pos.col = self.col_after(col);
                        contents.extend(cell.contents().as_bytes());
                    }
                } else if erase.is_none() {
                    erase = Some((pos.col, attrs));
                }
            }
        }
        if let Some((prev_col, attrs)) = erase {
            let new_pos = crate::grid::Pos { row, col: prev_col };
            if wrapping
                && prev_pos.row + 1 == new_pos.row
                && prev_pos.col >= self.cols()
            {
                if new_pos.col > 0 {
                    contents.extend(
                        " ".repeat(usize::from(new_pos.col)).as_bytes(),
                    );
                } else {
                    contents.extend(b" ");
                    crate::term::Backspace.write_buf(contents);
                }
            } else {
                crate::term::MoveFromTo::new(prev_pos, new_pos)
                    .write_buf(contents);
            }
            prev_pos = new_pos;
            if &prev_attrs != attrs {
                attrs.write_escape_code_diff(contents, &prev_attrs);
                prev_attrs = *attrs;
            }
            crate::term::ClearRowForward.write_buf(contents);
        }

        // if this row is going from wrapped to not wrapped, we need to erase
        // and redraw the last character to break wrapping. if this row is
        // wrapped, we need to redraw the last character without erasing it to
        // position the cursor after the end of the line correctly so that
        // drawing the next line can just start writing and be wrapped.
        if (!self.wrapped && prev.wrapped) || (!prev.wrapped && self.wrapped)
        {
            let end_pos = if self.cells[usize::from(self.cols() - 1)]
                .is_wide_continuation()
            {
                crate::grid::Pos {
                    row,
                    col: self.cols() - 2,
                }
            } else {
                crate::grid::Pos {
                    row,
                    col: self.cols() - 1,
                }
            };
            crate::term::MoveFromTo::new(prev_pos, end_pos)
                .write_buf(contents);
            prev_pos = end_pos;
            if !self.wrapped {
                crate::term::EraseChar::new(1).write_buf(contents);
            }
            let end_cell = &self.cells[usize::from(end_pos.col)];
            if end_cell.has_contents() && self.joins_left(end_pos.col, start)
            {
                self.write_cell_apart(
                    contents,
                    row,
                    end_pos.col,
                    start,
                    self.wrapped,
                    &mut prev_pos,
                    &mut prev_attrs,
                    deferred,
                );
            } else if end_cell.has_contents() {
                let attrs = end_cell.attrs();
                if &prev_attrs != attrs {
                    attrs.write_escape_code_diff(contents, &prev_attrs);
                    prev_attrs = *attrs;
                }
                if self.wrapped && self.holds_cursor(end_pos.col) {
                    self.write_held_cell_waiting(
                        contents,
                        row,
                        end_pos.col,
                        &mut prev_pos,
                        &mut prev_attrs,
                        deferred,
                        false,
                    );
                } else {
                    contents.extend(end_cell.contents().as_bytes());
                    prev_pos.col = self.col_after(end_pos.col);
                }
            }
        }

        (prev_pos, prev_attrs)
    }
}

// Wraps the row the cursor is waiting to wrap after (`prev_pos` is past its
// end) without printing anything right after its last cell, and leaves the
// cursor in the first column of the next row: a space wraps the row, and a
// backspace returns over it. The character printed there next is printed with
// the cursor moved to the first column, so it does not join the last cell of
// the row above (Row::first_cell_joins), and it replaces the space. With the
// cursor not past the end, nothing is written.
fn write_wrap_apart(
    contents: &mut Vec<u8>,
    prev_pos: crate::grid::Pos,
    cols: u16,
) {
    if prev_pos.col >= cols {
        contents.push(b' ');
        crate::term::Backspace.write_buf(contents);
    }
}
