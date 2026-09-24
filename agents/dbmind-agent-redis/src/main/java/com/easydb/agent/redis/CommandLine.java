package com.dbmind.agent.redis;

import java.util.ArrayList;
import java.util.List;

/**
 * 把用户写的命令切成参数：`SET k "a b"` → `["SET", "k", "a b"]`。
 *
 * <p>为什么需要它：Redis 的值常常带空格（`SET msg "hello world"`），
 * 按空格裸切会把一条命令切错，而**切错的命令可能仍然执行成功**——比如
 * `SET k hello world` 会被当成参数过多的错误还好，但 `LPUSH q hello world`
 * 就变成了两条数据。宁可在这里严谨一点。
 *
 * <p>支持双引号与单引号，以及反斜杠转义（`\"` `\\` `\n` `\t`）。
 */
public final class CommandLine {

    private CommandLine() {
    }

    public static List<String> tokenize(String line) {
        List<String> tokens = new ArrayList<>();
        StringBuilder current = new StringBuilder();
        boolean inToken = false;
        char quote = 0;

        for (int i = 0; i < line.length(); i++) {
            char c = line.charAt(i);
            if (quote != 0) {
                if (c == '\\' && i + 1 < line.length()) {
                    current.append(unescape(line.charAt(++i)));
                }
                else if (c == quote) {
                    quote = 0;
                }
                else {
                    current.append(c);
                }
                continue;
            }
            if (c == '"' || c == '\'') {
                quote = c;
                inToken = true;
                continue;
            }
            if (Character.isWhitespace(c)) {
                if (inToken) {
                    tokens.add(current.toString());
                    current.setLength(0);
                    inToken = false;
                }
                continue;
            }
            current.append(c);
            inToken = true;
        }
        if (inToken) {
            tokens.add(current.toString());
        }
        return tokens;
    }

    private static char unescape(char next) {
        return switch (next) {
            case 'n' -> '\n';
            case 'r' -> '\r';
            case 't' -> '\t';
            default -> next;
        };
    }
}
