use super::Theme;
use ratatui::style::Color;

impl Theme {
    pub fn default_dark() -> Self {
        Theme {
            name: "dark".to_string(),
            background: Color::Black,
            foreground: Color::White,
            primary: Color::Cyan,
            secondary: Color::Gray,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            info: Color::Blue,
            border: Color::DarkGray,
            highlight: Color::Yellow,
            comment: Color::DarkGray,
            string: Color::Green,
            keyword: Color::Magenta,
            function: Color::Cyan,
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(100, 255, 255),    // 更亮的青色
            secondary_shimmer: Color::Rgb(200, 200, 200),  // 更亮的灰色
            warning_shimmer: Color::Rgb(255, 255, 100),    // 更亮的黄色
            error_shimmer: Color::Rgb(255, 150, 150),      // 更亮的红色
            
            // Diff 颜色
            diff_added: Color::Rgb(34, 92, 43),            // 深绿色
            diff_removed: Color::Rgb(122, 41, 54),         // 深红色
            diff_added_dimmed: Color::Rgb(71, 88, 74),     // 暗绿色
            diff_removed_dimmed: Color::Rgb(105, 72, 77),  // 暗红色
            diff_added_word: Color::Rgb(56, 166, 96),      // 中绿色
            diff_removed_word: Color::Rgb(179, 89, 107),   // 中红色
            
            // Agent 颜色
            agent_red: Color::Rgb(220, 38, 38),            // Red 600
            agent_blue: Color::Rgb(37, 99, 235),           // Blue 600
            agent_green: Color::Rgb(22, 163, 74),          // Green 600
            agent_yellow: Color::Rgb(202, 138, 4),         // Yellow 600
            agent_purple: Color::Rgb(147, 51, 234),        // Purple 600
            agent_orange: Color::Rgb(234, 88, 12),         // Orange 600
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(55, 55, 55),       // 深灰色背景
            selection_bg: Color::Rgb(38, 79, 120),         // 蓝色选择背景
            inactive: Color::Rgb(153, 153, 153),           // 浅灰色
            subtle: Color::Rgb(80, 80, 80),                // 深灰色
            suggestion: Color::Rgb(177, 185, 249),         // 浅蓝紫色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(150, 150, 150),        // 灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(180, 180, 180),            // 浅灰色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(80, 220, 100),        // 绿色
            tool_error: Color::Rgb(255, 80, 80),           // 红色
            tool_border: Color::Rgb(253, 93, 177),         // 热粉色 #fd5db1
            
            // 用户消息颜色
            user_fg: Color::Rgb(100, 180, 255),            // 蓝色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(200, 200, 200),       // 浅灰色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(150, 150, 150),          // 灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(200, 200, 200),           // 浅灰色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(100, 100, 100),       // 深灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(180, 200, 140),            // 浅绿色
            code_bg: Color::Rgb(40, 40, 40),               // 深灰色
            
            // 链接颜色
            link_fg: Color::Rgb(100, 180, 255),            // 蓝色
        }
    }

    pub fn default_light() -> Self {
        Theme {
            name: "light".to_string(),
            background: Color::White,
            foreground: Color::Black,
            primary: Color::Blue,
            secondary: Color::Gray,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            info: Color::Cyan,
            border: Color::Gray,
            highlight: Color::Yellow,
            comment: Color::Gray,
            string: Color::Green,
            keyword: Color::Magenta,
            function: Color::Blue,
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(100, 130, 255),    // 更亮的蓝色
            secondary_shimmer: Color::Rgb(180, 180, 180),  // 更亮的灰色
            warning_shimmer: Color::Rgb(255, 200, 50),     // 更亮的黄色
            error_shimmer: Color::Rgb(255, 100, 100),      // 更亮的红色
            
            // Diff 颜色
            diff_added: Color::Rgb(105, 219, 124),         // 浅绿色
            diff_removed: Color::Rgb(255, 168, 180),       // 浅红色
            diff_added_dimmed: Color::Rgb(199, 225, 203),  // 非常浅的绿色
            diff_removed_dimmed: Color::Rgb(253, 210, 216),// 非常浅的红色
            diff_added_word: Color::Rgb(47, 157, 68),      // 中绿色
            diff_removed_word: Color::Rgb(209, 69, 75),    // 中红色
            
            // Agent 颜色
            agent_red: Color::Rgb(220, 38, 38),            // Red 600
            agent_blue: Color::Rgb(37, 99, 235),           // Blue 600
            agent_green: Color::Rgb(22, 163, 74),          // Green 600
            agent_yellow: Color::Rgb(202, 138, 4),         // Yellow 600
            agent_purple: Color::Rgb(147, 51, 234),        // Purple 600
            agent_orange: Color::Rgb(234, 88, 12),         // Orange 600
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(240, 240, 240),    // 浅灰色背景
            selection_bg: Color::Rgb(180, 213, 255),       // 浅蓝色选择背景
            inactive: Color::Rgb(102, 102, 102),           // 深灰色
            subtle: Color::Rgb(175, 175, 175),             // 浅灰色
            suggestion: Color::Rgb(87, 105, 247),          // 中蓝色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(120, 120, 120),        // 深灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(80, 80, 80),               // 深灰色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(40, 160, 60),         // 深绿色
            tool_error: Color::Rgb(200, 40, 40),           // 深红色
            tool_border: Color::Rgb(200, 50, 120),         // 深粉色
            
            // 用户消息颜色
            user_fg: Color::Rgb(30, 100, 200),             // 深蓝色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(60, 60, 60),          // 深灰色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(100, 100, 100),          // 深灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(60, 60, 60),              // 深灰色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(180, 180, 180),       // 浅灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(40, 100, 60),              // 深绿色
            code_bg: Color::Rgb(240, 240, 240),            // 浅灰色
            
            // 链接颜色
            link_fg: Color::Rgb(30, 100, 200),             // 深蓝色
        }
    }

    pub fn monokai() -> Self {
        Theme {
            name: "monokai".to_string(),
            background: Color::Rgb(39, 40, 34),
            foreground: Color::Rgb(248, 248, 242),
            primary: Color::Rgb(166, 226, 46),
            secondary: Color::Rgb(117, 113, 94),
            success: Color::Rgb(166, 226, 46),
            warning: Color::Rgb(253, 151, 31),
            error: Color::Rgb(249, 38, 114),
            info: Color::Rgb(102, 217, 239),
            border: Color::Rgb(117, 113, 94),
            highlight: Color::Rgb(253, 151, 31),
            comment: Color::Rgb(117, 113, 94),
            string: Color::Rgb(230, 219, 116),
            keyword: Color::Rgb(249, 38, 114),
            function: Color::Rgb(166, 226, 46),
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(200, 255, 100),    // 更亮的绿色
            secondary_shimmer: Color::Rgb(160, 155, 140),  // 更亮的灰色
            warning_shimmer: Color::Rgb(255, 200, 100),    // 更亮的橙色
            error_shimmer: Color::Rgb(255, 100, 160),      // 更亮的粉色
            
            // Diff 颜色
            diff_added: Color::Rgb(166, 226, 46),          // Monokai 绿色
            diff_removed: Color::Rgb(249, 38, 114),        // Monokai 粉色
            diff_added_dimmed: Color::Rgb(100, 140, 30),   // 暗绿色
            diff_removed_dimmed: Color::Rgb(150, 25, 70),  // 暗粉色
            diff_added_word: Color::Rgb(200, 255, 80),     // 亮绿色
            diff_removed_word: Color::Rgb(255, 80, 150),   // 亮粉色
            
            // Agent 颜色
            agent_red: Color::Rgb(249, 38, 114),           // Monokai 粉色
            agent_blue: Color::Rgb(102, 217, 239),         // Monokai 青色
            agent_green: Color::Rgb(166, 226, 46),         // Monokai 绿色
            agent_yellow: Color::Rgb(230, 219, 116),       // Monokai 黄色
            agent_purple: Color::Rgb(174, 129, 255),       // Monokai 紫色
            agent_orange: Color::Rgb(253, 151, 31),        // Monokai 橙色
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(55, 56, 50),       // 深灰色背景
            selection_bg: Color::Rgb(60, 80, 100),         // 深蓝色选择背景
            inactive: Color::Rgb(117, 113, 94),            // Monokai 灰色
            subtle: Color::Rgb(80, 80, 70),                // 深灰色
            suggestion: Color::Rgb(102, 217, 239),         // Monokai 青色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(117, 113, 94),         // Monokai 灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(248, 248, 242),            // Monokai 白色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(166, 226, 46),        // Monokai 绿色
            tool_error: Color::Rgb(249, 38, 114),          // Monokai 粉色
            tool_border: Color::Rgb(253, 93, 177),         // 热粉色
            
            // 用户消息颜色
            user_fg: Color::Rgb(102, 217, 239),            // Monokai 青色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(248, 248, 242),       // Monokai 白色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(117, 113, 94),           // Monokai 灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(248, 248, 242),           // Monokai 白色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(117, 113, 94),        // Monokai 灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(166, 226, 46),             // Monokai 绿色
            code_bg: Color::Rgb(55, 56, 50),               // Monokai 深灰色
            
            // 链接颜色
            link_fg: Color::Rgb(102, 217, 239),            // Monokai 青色
        }
    }

    pub fn dracula() -> Self {
        Theme {
            name: "dracula".to_string(),
            background: Color::Rgb(40, 42, 54),
            foreground: Color::Rgb(248, 248, 242),
            primary: Color::Rgb(189, 147, 249),
            secondary: Color::Rgb(98, 114, 164),
            success: Color::Rgb(80, 250, 123),
            warning: Color::Rgb(255, 184, 108),
            error: Color::Rgb(255, 85, 85),
            info: Color::Rgb(139, 233, 253),
            border: Color::Rgb(98, 114, 164),
            highlight: Color::Rgb(255, 184, 108),
            comment: Color::Rgb(98, 114, 164),
            string: Color::Rgb(241, 250, 140),
            keyword: Color::Rgb(255, 121, 198),
            function: Color::Rgb(80, 250, 123),
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(220, 190, 255),    // 更亮的紫色
            secondary_shimmer: Color::Rgb(140, 155, 200),  // 更亮的灰色
            warning_shimmer: Color::Rgb(255, 220, 170),    // 更亮的橙色
            error_shimmer: Color::Rgb(255, 140, 140),      // 更亮的红色
            
            // Diff 颜色
            diff_added: Color::Rgb(80, 250, 123),          // Dracula 绿色
            diff_removed: Color::Rgb(255, 85, 85),         // Dracula 红色
            diff_added_dimmed: Color::Rgb(50, 150, 75),    // 暗绿色
            diff_removed_dimmed: Color::Rgb(150, 50, 50),  // 暗红色
            diff_added_word: Color::Rgb(120, 255, 170),    // 亮绿色
            diff_removed_word: Color::Rgb(255, 130, 130),  // 亮红色
            
            // Agent 颜色
            agent_red: Color::Rgb(255, 85, 85),            // Dracula 红色
            agent_blue: Color::Rgb(139, 233, 253),         // Dracula 青色
            agent_green: Color::Rgb(80, 250, 123),         // Dracula 绿色
            agent_yellow: Color::Rgb(241, 250, 140),       // Dracula 黄色
            agent_purple: Color::Rgb(189, 147, 249),       // Dracula 紫色
            agent_orange: Color::Rgb(255, 184, 108),       // Dracula 橙色
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(55, 58, 72),       // 深灰色背景
            selection_bg: Color::Rgb(68, 71, 90),          // 深紫色选择背景
            inactive: Color::Rgb(98, 114, 164),            // Dracula 灰色
            subtle: Color::Rgb(68, 71, 90),                // 深灰色
            suggestion: Color::Rgb(189, 147, 249),         // Dracula 紫色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(98, 114, 164),         // Dracula 灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(248, 248, 242),            // Dracula 白色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(80, 250, 123),        // Dracula 绿色
            tool_error: Color::Rgb(255, 85, 85),           // Dracula 红色
            tool_border: Color::Rgb(255, 121, 198),        // Dracula 粉色
            
            // 用户消息颜色
            user_fg: Color::Rgb(139, 233, 253),            // Dracula 青色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(248, 248, 242),       // Dracula 白色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(98, 114, 164),           // Dracula 灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(248, 248, 242),           // Dracula 白色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(98, 114, 164),        // Dracula 灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(80, 250, 123),             // Dracula 绿色
            code_bg: Color::Rgb(55, 58, 72),               // Dracula 深灰色
            
            // 链接颜色
            link_fg: Color::Rgb(139, 233, 253),            // Dracula 青色
        }
    }

    pub fn solarized_dark() -> Self {
        Theme {
            name: "solarized-dark".to_string(),
            background: Color::Rgb(0, 43, 54),
            foreground: Color::Rgb(131, 148, 150),
            primary: Color::Rgb(38, 139, 210),
            secondary: Color::Rgb(108, 113, 119),
            success: Color::Rgb(133, 153, 0),
            warning: Color::Rgb(181, 137, 0),
            error: Color::Rgb(220, 50, 47),
            info: Color::Rgb(38, 139, 210),
            border: Color::Rgb(108, 113, 119),
            highlight: Color::Rgb(181, 137, 0),
            comment: Color::Rgb(108, 113, 119),
            string: Color::Rgb(42, 161, 152),
            keyword: Color::Rgb(203, 75, 22),
            function: Color::Rgb(38, 139, 210),
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(80, 180, 255),     // 更亮的蓝色
            secondary_shimmer: Color::Rgb(150, 155, 160),  // 更亮的灰色
            warning_shimmer: Color::Rgb(220, 180, 50),     // 更亮的黄色
            error_shimmer: Color::Rgb(255, 100, 100),      // 更亮的红色
            
            // Diff 颜色
            diff_added: Color::Rgb(133, 153, 0),           // Solarized 绿色
            diff_removed: Color::Rgb(220, 50, 47),         // Solarized 红色
            diff_added_dimmed: Color::Rgb(80, 95, 0),      // 暗绿色
            diff_removed_dimmed: Color::Rgb(130, 30, 28),  // 暗红色
            diff_added_word: Color::Rgb(170, 200, 0),      // 亮绿色
            diff_removed_word: Color::Rgb(255, 100, 100),  // 亮红色
            
            // Agent 颜色
            agent_red: Color::Rgb(220, 50, 47),            // Solarized 红色
            agent_blue: Color::Rgb(38, 139, 210),          // Solarized 蓝色
            agent_green: Color::Rgb(133, 153, 0),          // Solarized 绿色
            agent_yellow: Color::Rgb(181, 137, 0),         // Solarized 黄色
            agent_purple: Color::Rgb(108, 92, 160),        // Solarized 紫色
            agent_orange: Color::Rgb(203, 75, 22),         // Solarized 橙色
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(7, 54, 66),        // 深蓝色背景
            selection_bg: Color::Rgb(15, 75, 95),          // 深蓝色选择背景
            inactive: Color::Rgb(108, 113, 119),           // Solarized 灰色
            subtle: Color::Rgb(88, 93, 99),                // 深灰色
            suggestion: Color::Rgb(38, 139, 210),          // Solarized 蓝色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(108, 113, 119),        // Solarized 灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(131, 148, 150),            // Solarized 前景色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(133, 153, 0),         // Solarized 绿色
            tool_error: Color::Rgb(220, 50, 47),           // Solarized 红色
            tool_border: Color::Rgb(211, 54, 130),         // Solarized magenta
            
            // 用户消息颜色
            user_fg: Color::Rgb(38, 139, 210),             // Solarized 蓝色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(131, 148, 150),       // Solarized 前景色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(108, 113, 119),          // Solarized 灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(131, 148, 150),           // Solarized 前景色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(108, 113, 119),       // Solarized 灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(133, 153, 0),              // Solarized 绿色
            code_bg: Color::Rgb(0, 43, 54),                // Solarized 深蓝色
            
            // 链接颜色
            link_fg: Color::Rgb(38, 139, 210),             // Solarized 蓝色
        }
    }

    pub fn solarized_light() -> Self {
        Theme {
            name: "solarized-light".to_string(),
            background: Color::Rgb(253, 246, 227),
            foreground: Color::Rgb(101, 123, 131),
            primary: Color::Rgb(38, 139, 210),
            secondary: Color::Rgb(147, 161, 161),
            success: Color::Rgb(133, 153, 0),
            warning: Color::Rgb(181, 137, 0),
            error: Color::Rgb(220, 50, 47),
            info: Color::Rgb(38, 139, 210),
            border: Color::Rgb(147, 161, 161),
            highlight: Color::Rgb(181, 137, 0),
            comment: Color::Rgb(147, 161, 161),
            string: Color::Rgb(42, 161, 152),
            keyword: Color::Rgb(203, 75, 22),
            function: Color::Rgb(38, 139, 210),
            
            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(80, 180, 255),     // 更亮的蓝色
            secondary_shimmer: Color::Rgb(190, 200, 200),  // 更亮的灰色
            warning_shimmer: Color::Rgb(220, 180, 50),     // 更亮的黄色
            error_shimmer: Color::Rgb(255, 100, 100),      // 更亮的红色
            
            // Diff 颜色
            diff_added: Color::Rgb(133, 153, 0),           // Solarized 绿色
            diff_removed: Color::Rgb(220, 50, 47),         // Solarized 红色
            diff_added_dimmed: Color::Rgb(180, 200, 100),  // 浅绿色
            diff_removed_dimmed: Color::Rgb(240, 150, 150),// 浅红色
            diff_added_word: Color::Rgb(100, 130, 0),      // 深绿色
            diff_removed_word: Color::Rgb(180, 40, 40),    // 深红色
            
            // Agent 颜色
            agent_red: Color::Rgb(220, 50, 47),            // Solarized 红色
            agent_blue: Color::Rgb(38, 139, 210),          // Solarized 蓝色
            agent_green: Color::Rgb(133, 153, 0),          // Solarized 绿色
            agent_yellow: Color::Rgb(181, 137, 0),         // Solarized 黄色
            agent_purple: Color::Rgb(108, 92, 160),        // Solarized 紫色
            agent_orange: Color::Rgb(203, 75, 22),         // Solarized 橙色
            
            // UI 元素颜色
            user_message_bg: Color::Rgb(238, 232, 213),    // 浅灰色背景
            selection_bg: Color::Rgb(180, 213, 255),       // 浅蓝色选择背景
            inactive: Color::Rgb(147, 161, 161),           // Solarized 灰色
            subtle: Color::Rgb(180, 190, 190),             // 浅灰色
            suggestion: Color::Rgb(38, 139, 210),          // Solarized 蓝色
            
            // Thinking 相关颜色
            thinking_fg: Color::Rgb(147, 161, 161),        // Solarized 灰色
            thinking_bg: Color::Reset,                     // 透明
            
            // Tool 相关颜色
            tool_fg: Color::Rgb(101, 123, 131),            // Solarized 前景色
            tool_bg: Color::Reset,                         // 透明
            tool_success: Color::Rgb(133, 153, 0),         // Solarized 绿色
            tool_error: Color::Rgb(220, 50, 47),           // Solarized 红色
            tool_border: Color::Rgb(211, 54, 130),         // Solarized magenta
            
            // 用户消息颜色
            user_fg: Color::Rgb(38, 139, 210),             // Solarized 蓝色
            user_bg: Color::Reset,                         // 透明
            
            // Assistant 消息颜色
            assistant_fg: Color::Rgb(101, 123, 131),       // Solarized 前景色
            assistant_bg: Color::Reset,                    // 透明
            
            // 状态栏颜色
            status_fg: Color::Rgb(147, 161, 161),          // Solarized 灰色
            status_bg: Color::Reset,                       // 透明
            
            // 输入框颜色
            input_fg: Color::Rgb(101, 123, 131),           // Solarized 前景色
            input_bg: Color::Reset,                        // 透明
            input_border: Color::Rgb(147, 161, 161),       // Solarized 灰色
            
            // 代码块颜色
            code_fg: Color::Rgb(133, 153, 0),              // Solarized 绿色
            code_bg: Color::Rgb(253, 246, 227),            // Solarized 浅色
            
            // 链接颜色
            link_fg: Color::Rgb(38, 139, 210),             // Solarized 蓝色
        }
    }

    // ── Catppuccin Mocha (Dark) ─────────────────────────────────────
    pub fn catppuccin_mocha() -> Self {
        Theme {
            name: "catppuccin".to_string(),
            background: Color::Rgb(30, 30, 46),
            foreground: Color::Rgb(205, 214, 244),
            primary: Color::Rgb(203, 166, 247),     // Mauve
            secondary: Color::Rgb(108, 112, 134),   // Overlay1
            success: Color::Rgb(166, 227, 161),     // Green
            warning: Color::Rgb(249, 226, 175),     // Yellow
            error: Color::Rgb(243, 139, 168),       // Red
            info: Color::Rgb(137, 180, 250),        // Blue
            border: Color::Rgb(69, 71, 90),         // Surface1
            highlight: Color::Rgb(249, 226, 175),   // Yellow
            comment: Color::Rgb(108, 112, 134),     // Overlay1
            string: Color::Rgb(166, 227, 161),      // Green
            keyword: Color::Rgb(203, 166, 247),     // Mauve
            function: Color::Rgb(137, 180, 250),    // Blue
            primary_shimmer: Color::Rgb(220, 200, 255),
            secondary_shimmer: Color::Rgb(140, 140, 160),
            warning_shimmer: Color::Rgb(255, 240, 200),
            error_shimmer: Color::Rgb(255, 180, 200),
            diff_added: Color::Rgb(166, 227, 161),
            diff_removed: Color::Rgb(243, 139, 168),
            diff_added_dimmed: Color::Rgb(100, 150, 100),
            diff_removed_dimmed: Color::Rgb(150, 80, 100),
            diff_added_word: Color::Rgb(200, 255, 200),
            diff_removed_word: Color::Rgb(255, 180, 200),
            agent_red: Color::Rgb(243, 139, 168),
            agent_blue: Color::Rgb(137, 180, 250),
            agent_green: Color::Rgb(166, 227, 161),
            agent_yellow: Color::Rgb(249, 226, 175),
            agent_purple: Color::Rgb(203, 166, 247),
            agent_orange: Color::Rgb(250, 179, 135),
            user_message_bg: Color::Rgb(49, 50, 68),
            selection_bg: Color::Rgb(58, 60, 80),
            inactive: Color::Rgb(108, 112, 134),
            subtle: Color::Rgb(88, 91, 112),
            suggestion: Color::Rgb(137, 180, 250),
            thinking_fg: Color::Rgb(108, 112, 134),
            thinking_bg: Color::Reset,
            tool_fg: Color::Rgb(205, 214, 244),
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(166, 227, 161),
            tool_error: Color::Rgb(243, 139, 168),
            tool_border: Color::Rgb(245, 194, 231),        // Catppuccin pink
            user_fg: Color::Rgb(137, 180, 250),
            user_bg: Color::Reset,
            assistant_fg: Color::Rgb(205, 214, 244),
            assistant_bg: Color::Reset,
            status_fg: Color::Rgb(108, 112, 134),
            status_bg: Color::Reset,
            input_fg: Color::Rgb(205, 214, 244),
            input_bg: Color::Reset,
            input_border: Color::Rgb(69, 71, 90),
            code_fg: Color::Rgb(166, 227, 161),
            code_bg: Color::Rgb(49, 50, 68),
            link_fg: Color::Rgb(137, 180, 250),
        }
    }

    // ── Tokyo Night ─────────────────────────────────────────────────
    pub fn tokyo_night() -> Self {
        Theme {
            name: "tokyo-night".to_string(),
            background: Color::Rgb(26, 27, 38),
            foreground: Color::Rgb(192, 202, 245),
            primary: Color::Rgb(187, 154, 247),     // Magenta
            secondary: Color::Rgb(86, 95, 137),     // Comment
            success: Color::Rgb(158, 206, 106),     // Green
            warning: Color::Rgb(224, 175, 104),     // Yellow
            error: Color::Rgb(247, 118, 142),       // Red
            info: Color::Rgb(122, 162, 247),        // Blue
            border: Color::Rgb(59, 64, 91),         // Dark border
            highlight: Color::Rgb(224, 175, 104),   // Yellow
            comment: Color::Rgb(86, 95, 137),       // Comment
            string: Color::Rgb(158, 206, 106),      // Green
            keyword: Color::Rgb(187, 154, 247),     // Magenta
            function: Color::Rgb(122, 162, 247),    // Blue
            primary_shimmer: Color::Rgb(210, 190, 255),
            secondary_shimmer: Color::Rgb(120, 130, 170),
            warning_shimmer: Color::Rgb(255, 220, 150),
            error_shimmer: Color::Rgb(255, 160, 180),
            diff_added: Color::Rgb(158, 206, 106),
            diff_removed: Color::Rgb(247, 118, 142),
            diff_added_dimmed: Color::Rgb(90, 130, 60),
            diff_removed_dimmed: Color::Rgb(150, 70, 85),
            diff_added_word: Color::Rgb(190, 240, 140),
            diff_removed_word: Color::Rgb(255, 160, 180),
            agent_red: Color::Rgb(247, 118, 142),
            agent_blue: Color::Rgb(122, 162, 247),
            agent_green: Color::Rgb(158, 206, 106),
            agent_yellow: Color::Rgb(224, 175, 104),
            agent_purple: Color::Rgb(187, 154, 247),
            agent_orange: Color::Rgb(255, 158, 100),
            user_message_bg: Color::Rgb(40, 42, 58),
            selection_bg: Color::Rgb(40, 52, 87),
            inactive: Color::Rgb(86, 95, 137),
            subtle: Color::Rgb(65, 72, 105),
            suggestion: Color::Rgb(122, 162, 247),
            thinking_fg: Color::Rgb(86, 95, 137),
            thinking_bg: Color::Reset,
            tool_fg: Color::Rgb(192, 202, 245),
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(158, 206, 106),
            tool_error: Color::Rgb(247, 118, 142),
            tool_border: Color::Rgb(255, 158, 220),        // Tokyo Night pink
            user_fg: Color::Rgb(122, 162, 247),
            user_bg: Color::Reset,
            assistant_fg: Color::Rgb(192, 202, 245),
            assistant_bg: Color::Reset,
            status_fg: Color::Rgb(86, 95, 137),
            status_bg: Color::Reset,
            input_fg: Color::Rgb(192, 202, 245),
            input_bg: Color::Reset,
            input_border: Color::Rgb(59, 64, 91),
            code_fg: Color::Rgb(158, 206, 106),
            code_bg: Color::Rgb(40, 42, 58),
            link_fg: Color::Rgb(122, 162, 247),
        }
    }

    // ── Gruvbox Dark ────────────────────────────────────────────────
    pub fn gruvbox_dark() -> Self {
        Theme {
            name: "gruvbox".to_string(),
            background: Color::Rgb(40, 40, 40),
            foreground: Color::Rgb(235, 219, 178),
            primary: Color::Rgb(215, 153, 33),      // Yellow
            secondary: Color::Rgb(146, 131, 116),   // Gray
            success: Color::Rgb(184, 187, 38),      // Green
            warning: Color::Rgb(250, 189, 47),      // Yellow
            error: Color::Rgb(251, 73, 52),         // Red
            info: Color::Rgb(131, 165, 152),        // Blue
            border: Color::Rgb(146, 131, 116),      // Gray
            highlight: Color::Rgb(250, 189, 47),    // Yellow
            comment: Color::Rgb(146, 131, 116),     // Gray
            string: Color::Rgb(184, 187, 38),       // Green
            keyword: Color::Rgb(251, 73, 52),       // Red
            function: Color::Rgb(131, 165, 152),    // Blue
            primary_shimmer: Color::Rgb(255, 220, 100),
            secondary_shimmer: Color::Rgb(180, 165, 150),
            warning_shimmer: Color::Rgb(255, 235, 100),
            error_shimmer: Color::Rgb(255, 130, 110),
            diff_added: Color::Rgb(184, 187, 38),
            diff_removed: Color::Rgb(251, 73, 52),
            diff_added_dimmed: Color::Rgb(120, 120, 25),
            diff_removed_dimmed: Color::Rgb(160, 45, 35),
            diff_added_word: Color::Rgb(220, 225, 60),
            diff_removed_word: Color::Rgb(255, 120, 100),
            agent_red: Color::Rgb(251, 73, 52),
            agent_blue: Color::Rgb(131, 165, 152),
            agent_green: Color::Rgb(184, 187, 38),
            agent_yellow: Color::Rgb(215, 153, 33),
            agent_purple: Color::Rgb(177, 90, 69),
            agent_orange: Color::Rgb(254, 128, 25),
            user_message_bg: Color::Rgb(60, 56, 54),
            selection_bg: Color::Rgb(80, 73, 69),
            inactive: Color::Rgb(146, 131, 116),
            subtle: Color::Rgb(100, 90, 78),
            suggestion: Color::Rgb(131, 165, 152),
            thinking_fg: Color::Rgb(146, 131, 116),
            thinking_bg: Color::Reset,
            tool_fg: Color::Rgb(235, 219, 178),
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(184, 187, 38),
            tool_error: Color::Rgb(251, 73, 52),
            tool_border: Color::Rgb(254, 128, 25),         // Gruvbox orange
            user_fg: Color::Rgb(131, 165, 152),
            user_bg: Color::Reset,
            assistant_fg: Color::Rgb(235, 219, 178),
            assistant_bg: Color::Reset,
            status_fg: Color::Rgb(146, 131, 116),
            status_bg: Color::Reset,
            input_fg: Color::Rgb(235, 219, 178),
            input_bg: Color::Reset,
            input_border: Color::Rgb(146, 131, 116),
            code_fg: Color::Rgb(184, 187, 38),
            code_bg: Color::Rgb(60, 56, 54),
            link_fg: Color::Rgb(131, 165, 152),
        }
    }

    // ── Nord ────────────────────────────────────────────────────────
    pub fn nord() -> Self {
        Theme {
            name: "nord".to_string(),
            background: Color::Rgb(46, 52, 64),
            foreground: Color::Rgb(216, 222, 233),
            primary: Color::Rgb(136, 192, 208),     // Frost
            secondary: Color::Rgb(76, 86, 106),     // Polar Night
            success: Color::Rgb(163, 190, 140),     // Aurora Green
            warning: Color::Rgb(235, 203, 139),     // Aurora Yellow
            error: Color::Rgb(191, 97, 106),        // Aurora Red
            info: Color::Rgb(129, 161, 193),        // Frost
            border: Color::Rgb(76, 86, 106),        // Polar Night
            highlight: Color::Rgb(235, 203, 139),   // Aurora Yellow
            comment: Color::Rgb(76, 86, 106),       // Polar Night
            string: Color::Rgb(163, 190, 140),      // Aurora Green
            keyword: Color::Rgb(180, 142, 173),     // Aurora Purple
            function: Color::Rgb(136, 192, 208),    // Frost
            primary_shimmer: Color::Rgb(180, 220, 235),
            secondary_shimmer: Color::Rgb(110, 120, 140),
            warning_shimmer: Color::Rgb(255, 235, 180),
            error_shimmer: Color::Rgb(230, 150, 160),
            diff_added: Color::Rgb(163, 190, 140),
            diff_removed: Color::Rgb(191, 97, 106),
            diff_added_dimmed: Color::Rgb(100, 120, 85),
            diff_removed_dimmed: Color::Rgb(120, 60, 65),
            diff_added_word: Color::Rgb(200, 230, 180),
            diff_removed_word: Color::Rgb(230, 150, 160),
            agent_red: Color::Rgb(191, 97, 106),
            agent_blue: Color::Rgb(129, 161, 193),
            agent_green: Color::Rgb(163, 190, 140),
            agent_yellow: Color::Rgb(235, 203, 139),
            agent_purple: Color::Rgb(180, 142, 173),
            agent_orange: Color::Rgb(208, 135, 112),
            user_message_bg: Color::Rgb(59, 66, 82),
            selection_bg: Color::Rgb(67, 76, 94),
            inactive: Color::Rgb(76, 86, 106),
            subtle: Color::Rgb(67, 76, 94),
            suggestion: Color::Rgb(136, 192, 208),
            thinking_fg: Color::Rgb(76, 86, 106),
            thinking_bg: Color::Reset,
            tool_fg: Color::Rgb(216, 222, 233),
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(163, 190, 140),
            tool_error: Color::Rgb(191, 97, 106),
            tool_border: Color::Rgb(180, 142, 173),        // Nord pink
            user_fg: Color::Rgb(136, 192, 208),
            user_bg: Color::Reset,
            assistant_fg: Color::Rgb(216, 222, 233),
            assistant_bg: Color::Reset,
            status_fg: Color::Rgb(76, 86, 106),
            status_bg: Color::Reset,
            input_fg: Color::Rgb(216, 222, 233),
            input_bg: Color::Reset,
            input_border: Color::Rgb(76, 86, 106),
            code_fg: Color::Rgb(163, 190, 140),
            code_bg: Color::Rgb(59, 66, 82),
            link_fg: Color::Rgb(136, 192, 208),
        }
    }

    // ── One Dark ────────────────────────────────────────────────────
    pub fn one_dark() -> Self {
        Theme {
            name: "one-dark".to_string(),
            background: Color::Rgb(40, 44, 52),
            foreground: Color::Rgb(171, 178, 191),
            primary: Color::Rgb(97, 175, 239),      // Blue
            secondary: Color::Rgb(92, 99, 112),     // Comment Grey
            success: Color::Rgb(152, 195, 121),     // Green
            warning: Color::Rgb(229, 192, 123),     // Yellow
            error: Color::Rgb(224, 108, 117),       // Red
            info: Color::Rgb(97, 175, 239),         // Blue
            border: Color::Rgb(92, 99, 112),        // Comment Grey
            highlight: Color::Rgb(229, 192, 123),   // Yellow
            comment: Color::Rgb(92, 99, 112),       // Comment Grey
            string: Color::Rgb(152, 195, 121),      // Green
            keyword: Color::Rgb(198, 120, 221),     // Magenta
            function: Color::Rgb(97, 175, 239),     // Blue
            primary_shimmer: Color::Rgb(140, 200, 255),
            secondary_shimmer: Color::Rgb(130, 135, 145),
            warning_shimmer: Color::Rgb(255, 225, 160),
            error_shimmer: Color::Rgb(255, 160, 170),
            diff_added: Color::Rgb(152, 195, 121),
            diff_removed: Color::Rgb(224, 108, 117),
            diff_added_dimmed: Color::Rgb(90, 125, 70),
            diff_removed_dimmed: Color::Rgb(140, 65, 75),
            diff_added_word: Color::Rgb(190, 235, 160),
            diff_removed_word: Color::Rgb(255, 160, 170),
            agent_red: Color::Rgb(224, 108, 117),
            agent_blue: Color::Rgb(97, 175, 239),
            agent_green: Color::Rgb(152, 195, 121),
            agent_yellow: Color::Rgb(229, 192, 123),
            agent_purple: Color::Rgb(198, 120, 221),
            agent_orange: Color::Rgb(255, 150, 100),
            user_message_bg: Color::Rgb(55, 60, 72),
            selection_bg: Color::Rgb(62, 68, 81),
            inactive: Color::Rgb(92, 99, 112),
            subtle: Color::Rgb(76, 82, 95),
            suggestion: Color::Rgb(97, 175, 239),
            thinking_fg: Color::Rgb(92, 99, 112),
            thinking_bg: Color::Reset,
            tool_fg: Color::Rgb(171, 178, 191),
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(152, 195, 121),
            tool_error: Color::Rgb(224, 108, 117),
            tool_border: Color::Rgb(198, 120, 221),        // One Dark purple
            user_fg: Color::Rgb(97, 175, 239),
            user_bg: Color::Reset,
            assistant_fg: Color::Rgb(171, 178, 191),
            assistant_bg: Color::Reset,
            status_fg: Color::Rgb(92, 99, 112),
            status_bg: Color::Reset,
            input_fg: Color::Rgb(171, 178, 191),
            input_bg: Color::Reset,
            input_border: Color::Rgb(92, 99, 112),
            code_fg: Color::Rgb(152, 195, 121),
            code_bg: Color::Rgb(55, 60, 72),
            link_fg: Color::Rgb(97, 175, 239),
        }
    }

    /// Claude Code 风格主题 — 温暖赤陶色、热粉色工具边框、薰衣草权限框
    pub fn claude_code() -> Self {
        Theme {
            name: "claude-code".to_string(),
            background: Color::Rgb(26, 26, 26),        // #1a1a1a
            foreground: Color::White,                    // #ffffff
            primary: Color::Rgb(215, 119, 87),           // #d77757 赤陶色
            secondary: Color::Rgb(253, 93, 177),         // #fd5db1 热粉色
            success: Color::Rgb(78, 186, 101),           // #4eba65
            warning: Color::Rgb(255, 193, 7),            // #ffc107
            error: Color::Rgb(255, 107, 128),            // #ff6b80
            info: Color::Rgb(177, 185, 249),             // #b1b9f9 薰衣草色
            border: Color::Rgb(136, 136, 136),           // #888888
            highlight: Color::Rgb(255, 193, 7),          // #ffc107
            comment: Color::Rgb(136, 136, 136),          // #888888
            string: Color::Rgb(78, 186, 101),            // #4eba65
            keyword: Color::Rgb(215, 119, 87),           // #d77757
            function: Color::Rgb(177, 185, 249),         // #b1b9f9

            // Shimmer 效果
            primary_shimmer: Color::Rgb(235, 159, 127),  // #eb9f7f 浅赤陶色
            secondary_shimmer: Color::Rgb(255, 140, 200), // 浅热粉色
            warning_shimmer: Color::Rgb(255, 220, 100),
            error_shimmer: Color::Rgb(255, 150, 150),

            // Diff 颜色
            diff_added: Color::Rgb(34, 92, 43),          // #225c2b
            diff_removed: Color::Rgb(122, 41, 54),       // #7a2936
            diff_added_dimmed: Color::Rgb(50, 80, 55),
            diff_removed_dimmed: Color::Rgb(100, 60, 68),
            diff_added_word: Color::Rgb(56, 166, 96),
            diff_removed_word: Color::Rgb(179, 89, 107),

            // Agent 颜色
            agent_red: Color::Rgb(220, 38, 38),
            agent_blue: Color::Rgb(37, 99, 235),
            agent_green: Color::Rgb(22, 163, 74),
            agent_yellow: Color::Rgb(202, 138, 4),
            agent_purple: Color::Rgb(175, 135, 255),      // #af87ff Auto-accept 紫色
            agent_orange: Color::Rgb(234, 88, 12),

            // UI 元素
            user_message_bg: Color::Rgb(55, 55, 55),     // #373737 Surface
            selection_bg: Color::Rgb(38, 79, 120),
            inactive: Color::Rgb(153, 153, 153),          // #999999
            subtle: Color::Rgb(80, 80, 80),               // #505050
            suggestion: Color::Rgb(177, 185, 249),        // #b1b9f9

            // Thinking 颜色
            thinking_fg: Color::Rgb(215, 119, 87),        // 赤陶色（同 primary）
            thinking_bg: Color::Reset,

            // Tool 颜色 — 热粉色边框
            tool_fg: Color::Rgb(180, 180, 180),
            tool_bg: Color::Rgb(65, 60, 65),              // 工具块背景
            tool_success: Color::Rgb(78, 186, 101),       // #4eba65
            tool_error: Color::Rgb(255, 107, 128),        // #ff6b80
            tool_border: Color::Rgb(253, 93, 177),         // 热粉色 #fd5db1

            // 用户消息
            user_fg: Color::White,
            user_bg: Color::Reset,

            // Assistant 消息
            assistant_fg: Color::White,                   // 纯白色响应
            assistant_bg: Color::Reset,

            // 状态栏
            status_fg: Color::Rgb(136, 136, 136),         // #888888 Muted
            status_bg: Color::Reset,

            // 输入框
            input_fg: Color::Rgb(200, 200, 200),
            input_bg: Color::Rgb(55, 55, 55),             // #373737 Surface
            input_border: Color::Rgb(136, 136, 136),      // #888888 Muted

            // 代码块
            code_fg: Color::Rgb(180, 200, 140),
            code_bg: Color::Rgb(40, 40, 40),

            // 链接
            link_fg: Color::Rgb(100, 180, 255),
        }
    }

    /// 高对比度无障碍主题 — 最大化可读性，适合视力受限用户
    pub fn high_contrast() -> Self {
        Theme {
            name: "high-contrast".to_string(),
            background: Color::Black,
            foreground: Color::White,
            primary: Color::Rgb(0, 255, 255),       // 纯青色
            secondary: Color::White,
            success: Color::Rgb(0, 255, 0),          // 纯绿色
            warning: Color::Rgb(255, 255, 0),        // 纯黄色
            error: Color::Rgb(255, 0, 0),            // 纯红色
            info: Color::Rgb(0, 128, 255),           // 亮蓝色
            border: Color::White,                    // 白色边框（高对比）
            highlight: Color::Rgb(255, 255, 0),      // 纯黄色高亮
            comment: Color::Rgb(180, 180, 180),      // 浅灰色
            string: Color::Rgb(0, 255, 0),           // 纯绿色
            keyword: Color::Rgb(255, 0, 255),        // 纯洋红
            function: Color::Rgb(0, 255, 255),       // 纯青色

            // Shimmer 效果颜色
            primary_shimmer: Color::Rgb(128, 255, 255),
            secondary_shimmer: Color::Rgb(255, 255, 255),
            warning_shimmer: Color::Rgb(255, 255, 128),
            error_shimmer: Color::Rgb(255, 128, 128),

            // Diff 颜色
            diff_added: Color::Rgb(0, 180, 0),
            diff_removed: Color::Rgb(255, 60, 60),
            diff_added_dimmed: Color::Rgb(0, 100, 0),
            diff_removed_dimmed: Color::Rgb(150, 30, 30),
            diff_added_word: Color::Rgb(0, 255, 0),
            diff_removed_word: Color::Rgb(255, 100, 100),

            // Agent 颜色
            agent_red: Color::Rgb(255, 80, 80),
            agent_blue: Color::Rgb(80, 160, 255),
            agent_green: Color::Rgb(80, 255, 80),
            agent_yellow: Color::Rgb(255, 255, 80),
            agent_purple: Color::Rgb(200, 120, 255),
            agent_orange: Color::Rgb(255, 180, 80),

            // UI 元素颜色
            user_message_bg: Color::Rgb(30, 30, 60),      // 深蓝色背景
            selection_bg: Color::Rgb(0, 80, 160),          // 高对比选择背景
            inactive: Color::Rgb(180, 180, 180),
            subtle: Color::Rgb(120, 120, 120),
            suggestion: Color::Rgb(128, 200, 255),

            // Thinking 相关颜色
            thinking_fg: Color::Rgb(200, 200, 200),
            thinking_bg: Color::Reset,

            // Tool 相关颜色
            tool_fg: Color::White,
            tool_bg: Color::Reset,
            tool_success: Color::Rgb(0, 255, 0),
            tool_error: Color::Rgb(255, 80, 80),
            tool_border: Color::Rgb(255, 128, 255),

            // 用户消息颜色
            user_fg: Color::Rgb(128, 200, 255),
            user_bg: Color::Reset,

            // Assistant 消息颜色
            assistant_fg: Color::White,
            assistant_bg: Color::Reset,

            // 状态栏颜色
            status_fg: Color::Rgb(200, 200, 200),
            status_bg: Color::Reset,

            // 输入框颜色
            input_fg: Color::White,
            input_bg: Color::Reset,
            input_border: Color::White,               // 白色边框

            // 代码块颜色
            code_fg: Color::Rgb(0, 255, 0),            // 纯绿色
            code_bg: Color::Rgb(20, 20, 20),

            // 链接颜色
            link_fg: Color::Rgb(128, 200, 255),
        }
    }
}
