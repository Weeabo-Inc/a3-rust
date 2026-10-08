"""Best-effort demangler for MSVC RTTI type names (".?AVFoo@Bar@@" -> "Bar::Foo").

Covers what RTTI type descriptors contain: nested names, templates with type and
integer arguments, pointers/references, primitive types, and name back-references.
Anything it cannot parse is returned unchanged by `demangle_type_name`.
"""

from __future__ import annotations

PRIMITIVES = {
    "C": "signed char", "D": "char", "E": "unsigned char", "F": "short",
    "G": "unsigned short", "H": "int", "I": "unsigned int", "J": "long",
    "K": "unsigned long", "M": "float", "N": "double", "O": "long double",
    "X": "void", "Z": "...",
}
EXT_PRIMITIVES = {
    "_N": "bool", "_J": "__int64", "_K": "unsigned __int64", "_W": "wchar_t",
    "_S": "char16_t", "_U": "char32_t", "_Q": "char8_t", "_D": "__int8", "_E": "unsigned __int8",
    "_F": "__int16", "_G": "unsigned __int16", "_H": "__int32", "_I": "unsigned __int32",
}


class _Parser:
    def __init__(self, s: str):
        self.s = s
        self.i = 0
        self.names: list[str] = []  # back-reference table for name fragments

    def peek(self, n: int = 1) -> str:
        return self.s[self.i:self.i + n]

    def take(self, n: int = 1) -> str:
        r = self.s[self.i:self.i + n]
        if len(r) < n:
            raise ValueError("unexpected end")
        self.i += n
        return r

    def number(self) -> int:
        neg = False
        if self.peek() == "?":
            neg = True
            self.take()
        c = self.take()
        if c.isdigit():
            v = int(c) + 1
        else:
            v = 0
            while c != "@":
                v = v * 16 + (ord(c) - ord("A"))
                c = self.take()
        return -v if neg else v

    def fragment(self) -> str:
        c = self.peek()
        if c.isdigit():
            self.take()
            return self.names[int(c)]
        if self.peek(2) == "?$":
            self.take(2)
            # template: own back-reference scope
            saved = self.names
            self.names = []
            name = self.simple_name()
            self.names.append(name)
            args = []
            while self.peek() != "@":
                args.append(self.template_arg())
            self.take()  # '@' ends the argument list
            self.names = saved
            full = f"{name}<{','.join(args)}>"
            self.names.append(full)
            return full
        if self.peek(2) == "??":
            # scope is a function: ??name@@<signature>
            self.take()
            return self.nested_function()  # shares the enclosing back-reference table
        if self.peek() == "?":
            self.take()
            if self.peek(2) == "A0":
                # anonymous namespace ?A0x1234abcd@
                end = self.s.index("@", self.i)
                self.i = end + 1
                return "`anonymous namespace'"
            # numbered local scope ?<number>
            return f"`{self.number()}'"
        name = self.simple_name()
        if len(self.names) < 10:
            self.names.append(name)
        return name

    def simple_name(self) -> str:
        end = self.s.index("@", self.i)
        name = self.s[self.i:end]
        self.i = end + 1
        return name

    def qualified_name(self) -> str:
        parts = []
        while self.peek() != "@":
            parts.append(self.fragment())
        self.take()
        return "::".join(reversed(parts))

    def template_arg(self) -> str:
        if self.peek(2) == "$0":
            self.take(2)
            return str(self.number())
        if self.peek(2) in ("$1", "$E"):
            self.take(2)
            return "&" + self.mangled_symbol()
        if self.peek(2) == "$$":
            self.take(2)
            c = self.take()
            if c in ("Q", "R"):
                return self.type_() + "&&"
            if c == "C":
                return self.modifiers() + self.type_()
            if c == "V" or c == "Z":
                return ""
            return self.type_()
        return self.type_()

    def nested_function(self) -> str:
        """Parse `?name@@<function encoding>` and return `name()` (signature skipped)."""
        self.take()  # leading '?'
        name = self.qualified_name()
        c = self.take()
        if c == "Y":  # global function
            self.take()  # calling convention
        elif "A" <= c <= "V":
            static = c in "CDKLST"
            if not static:
                if self.peek() == "E":
                    self.take()
                self.take()  # cv of this
            self.take()  # calling convention
        else:
            raise ValueError("function kind " + c)
        # return type (may be '@' for ctor/dtor, '?A' cv prefix)
        if self.peek() == "@":
            self.take()
        else:
            if self.peek() == "?":
                self.take(2)
            self.type_()
        args: list[str] = []
        while True:
            c = self.peek()
            if c == "X":
                self.take()
                break
            if c == "@":
                self.take()
                break
            if c == "Z":
                break
            if c.isdigit():
                self.take()
                continue
            t = self.type_()
            if len(args) < 10:
                args.append(t)
        if self.peek() == "Z":
            self.take()
        return f"{name}()"

    def function_type(self) -> str:
        """Parse `<cc><ret><args>Z` (after `P6` / `$$A6`) and return `ret(args)`."""
        self.take()  # calling convention
        if self.peek() == "?":
            self.take(2)
        ret = self.type_()
        args: list[str] = []
        while True:
            c = self.peek()
            if c == "X" and not args:
                self.take()
                break
            if c in "@Z":
                if c == "@":
                    self.take()
                break
            if c.isdigit():
                self.take()
                args.append(args[int(c)] if int(c) < len(args) else "?")
                continue
            args.append(self.type_())
        if self.peek() == "Z":
            self.take()
        return f"{ret}({','.join(args)})"

    def mangled_symbol(self) -> str:
        if self.peek() == "?":
            self.take()
        name = self.qualified_name()
        # skip the rest of the symbol encoding; good enough for display
        while self.i < len(self.s) and self.peek() != "@":
            self.take()
        return name

    def modifiers(self) -> str:
        c = self.take()
        return {"A": "", "B": "const ", "C": "volatile ", "D": "const volatile "}.get(c, "")

    def type_(self) -> str:
        c = self.peek()
        if c == "_":
            code = self.take(2)
            if code in EXT_PRIMITIVES:
                return EXT_PRIMITIVES[code]
            raise ValueError("ext type " + code)
        if c in PRIMITIVES:
            self.take()
            return PRIMITIVES[c]
        if c in "VUT":
            self.take()
            return self.qualified_name()
        if c == "W":
            self.take(2)
            return "enum " + self.qualified_name()
        if c in "PQRS":
            self.take()
            if self.peek() == "6":
                self.take()
                return self.function_type() + "*"
            if self.peek() == "E":
                self.take()
            cv = self.modifiers()
            inner = self.type_()
            return f"{cv}{inner}*"
        if c == "A":
            self.take()
            if self.peek() == "E":
                self.take()
            cv = self.modifiers()
            return f"{cv}{self.type_()}&"
        if c == "$":
            if self.peek(3) == "$$Q":
                self.take(3)
                if self.peek() == "E":
                    self.take()
                cv = self.modifiers()
                return f"{cv}{self.type_()}&&"
            if self.peek(3) == "$$A":
                self.take(3)
                if self.peek() == "6":
                    self.take()
                return self.function_type()
            if self.peek(3) == "$$B":
                self.take(3)
                return self.type_()
        raise ValueError(f"type code {c!r} at {self.i}")


def demangle_type_name(mangled: str) -> str:
    """Demangle an RTTI TypeDescriptor name such as `.?AVEntity@@`."""
    s = mangled
    if s.startswith(".?A"):
        kind = s[3:4]
        body = s[4:]
        try:
            p = _Parser(body)
            if kind == "W":
                p.take()  # enum underlying-type digit
            return p.qualified_name()
        except (ValueError, IndexError):
            return mangled
    return mangled


if __name__ == "__main__":
    import sys

    for arg in sys.argv[1:]:
        print(demangle_type_name(arg))
