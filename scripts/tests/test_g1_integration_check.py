from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from g1_integration_check import executed_tests, run_step


class IntegrationEvidenceTests(unittest.TestCase):
    def test_rust_sums_only_executed_successes(self):
        log = ('test result: ok. 0 passed; 0 failed; 0 ignored;\n'
               'test result: ok. 43 passed; 0 failed; 0 ignored;\n'
               'test result: ok. 20 passed; 0 failed; 0 ignored;\n')
        self.assertEqual(executed_tests('rust-tests', log), 63)

    def test_swift_enclosing_suites_are_not_double_counted(self):
        log = ("Test Suite 'ProbeKitTests.xctest' passed at time.\n Executed 20 tests, with 0 failures\n"
               "Test Suite 'All tests' passed at time.\n Executed 20 tests, with 0 failures\n") * 2
        self.assertEqual(executed_tests('swift-tests', log), 20)

    def test_swift_multiple_bundles_are_summed_not_maximized(self):
        log = ("Test Suite 'ProbeKitTests.xctest' passed at time.\n Executed 20 tests, with 0 failures\n"
               "Test Suite 'OCRKitTests.xctest' passed at time.\n Executed 26 tests, with 0 failures\n")
        self.assertEqual(executed_tests('swift-tests', log), 46)
        self.assertEqual(executed_tests('swift-tests', 'Executed 46 tests, with 0 failures'), 0)

    def test_python_count_and_missing_summary(self):
        self.assertEqual(executed_tests('python-tests', 'Ran 23 tests in 0.2s\nOK'), 23)
        self.assertEqual(executed_tests('cipher-disabled', 'Finished build'), 0)

    def test_exit_zero_without_executed_test_proof_is_not_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            result = run_step('cipher-disabled', [sys.executable, '-c', 'print("no tests")'], Path(directory), timeout=5)
            self.assertEqual(result['exit_code'], 0)
            self.assertEqual(result['executed_tests'], 0)
            self.assertFalse(result['passed'])

    def test_zero_test_summary_does_not_satisfy_exact_filter(self):
        self.assertEqual(executed_tests('cipher-disabled', 'test result: ok. 0 passed; 0 failed;'), 0)


if __name__ == '__main__':
    unittest.main()
