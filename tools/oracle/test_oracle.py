"""Tests for the oracle harness: probe parsing, log parsing, diff. Run: python -m unittest
discover -s tools/oracle (stdlib only; no game data needed)."""

import unittest

import oracle


class ProbeFiles(unittest.TestCase):
    def test_probes_with_world_and_comments(self):
        text = "// header\n@world Stratis\n## a.one\nstr 1\n// dropped\n## a.two\nprivate _x = 1;\n_x\n"
        probes = oracle.parse_probe_file(text, "t.probes")
        self.assertEqual([p.id for p in probes], ["a.one", "a.two"])
        self.assertEqual(probes[1].code, "private _x = 1;\n_x")
        self.assertTrue(all(p.world == "Stratis" for p in probes))

    def test_code_outside_a_probe_is_an_error(self):
        with self.assertRaises(ValueError):
            oracle.parse_probe_file("str 1\n", "t.probes")

    def test_the_committed_corpus_parses_with_unique_ids(self):
        probes = oracle.load_corpus()
        self.assertGreater(len(probes), 300)
        self.assertEqual(len({p.id for p in probes}), len(probes))

    def test_driver_doubles_quotes(self):
        probe = oracle.Probe("q.x", 'str "a"', "t", None)
        self.assertIn('["q.x", "str ""a"""]', oracle.driver_script([probe]))


class Logs(unittest.TestCase):
    def test_rpt_stamps_are_stripped(self):
        lines = oracle.strip_rpt(" 4:31:16 A3RO_BEGIN|x\n12:01:02.123 A3RO|x|SCALAR|1\nError tail")
        self.assertEqual(lines, ["A3RO_BEGIN|x", "A3RO|x|SCALAR|1", "Error tail"])

    def test_results_values_numbers_and_evidence(self):
        lines = [
            "noise before",
            "A3RO_BEGIN|a",
            "A3RO|a|SCALAR|0.3",
            "A3RN|a|+1677722p-2",
            "A3RO_END|a|1",
            "A3RO_BEGIN|b",
            "Error in expression <1 + \"a\">",
            "A3RO_END|b|0",
            "A3RO_BEGIN|c",
            "A3RE|c|STRING|\"a\\nb\\\\c\"",
            "A3RO_END|c|1",
            "A3RO_BEGIN|d",
            "A3RV|d|ab",
            "A3RV|d|cd",
            "A3RL|d|STRING|2",
            "A3RO_END|d|1",
            "A3RO_DONE",
        ]
        results, done = oracle.parse_log(lines)
        self.assertTrue(done)
        self.assertEqual((results["a"].type, results["a"].value), ("SCALAR", "0.3"))
        self.assertEqual(results["a"].number, "+1677722p-2")
        self.assertEqual(results["a"].status, "ok")
        self.assertEqual(results["b"].status, "error")
        self.assertEqual(results["b"].evidence, ['Error in expression <1 + "a">'])
        self.assertEqual(results["c"].value, '"a\nb\\c"')
        self.assertEqual(results["d"].value, "abcd")

    def test_value_may_contain_the_separator(self):
        results, _ = oracle.parse_log(["A3RO_BEGIN|a", "A3RO|a|STRING|\"x|y\"", "A3RO_END|a|1"])
        self.assertEqual(results["a"].value, '"x|y"')


class Diff(unittest.TestCase):
    def result(self, type_, value, number=None, completed=True):
        r = oracle.Result("p", begun=True, ended=True, completed=completed)
        r.type, r.value, r.number = type_, value, number
        return r

    def test_categories(self):
        ok = self.result("SCALAR", "1", "+0p0")
        self.assertEqual(oracle.classify(ok, self.result("SCALAR", "1", "+0p0")), "match")
        self.assertEqual(oracle.classify(ok, self.result("SCALAR", "1", "+1p0")), "precision")
        self.assertEqual(oracle.classify(ok, self.result("STRING", "1")), "mismatch")
        failed = self.result(None, None, completed=False)
        self.assertEqual(oracle.classify(ok, failed), "ours-error")
        self.assertEqual(oracle.classify(failed, ok), "oracle-error")
        self.assertEqual(oracle.classify(failed, failed), "both-error")
        self.assertEqual(oracle.classify(None, ok), "missing")

    def test_summary_counts_areas(self):
        rows = [
            {"id": "num.a", "category": "match"},
            {"id": "num.b", "category": "mismatch"},
            {"id": "str.a", "category": "oracle-error"},
        ]
        md = oracle.summary_markdown(rows, {"date": "d", "commit": "c"})
        self.assertIn("**1 of 2 comparable probes match (50.0%)**", md)
        self.assertIn("| num | 1 | 1 |", md)
        self.assertIn("| `num.b` | mismatch |", md)


if __name__ == "__main__":
    unittest.main()
