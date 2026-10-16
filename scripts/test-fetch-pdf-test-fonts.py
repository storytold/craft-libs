#!/usr/bin/env python3
"""Exercise fixture-download retries without network access."""

import importlib.util
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import urllib.error

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("fetch_pdf_fonts", Path(__file__).with_name("fetch-pdf-test-fonts.py"))
fetcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fetcher)
URL = "https://example.invalid/font.ttf"


class FetchTests(unittest.TestCase):
    def test_success_needs_no_retry(self):
        with patch.object(fetcher.urllib.request, "urlopen", return_value=io.BytesIO(b"font")) as request:
            with patch.object(fetcher.time, "sleep") as sleep:
                self.assertEqual(fetcher.fetch(URL), b"font")
                request.assert_called_once_with(URL, timeout=60)
                sleep.assert_not_called()

    def test_transient_failures_retry_then_succeed(self):
        errors = [
            urllib.error.HTTPError(URL, 429, "rate limit", {}, None),
            urllib.error.HTTPError(URL, 503, "unavailable", {}, None),
            urllib.error.URLError("temporary DNS failure"),
            TimeoutError("timed out"),
            ConnectionResetError("reset"),
        ]
        for error in errors:
            if isinstance(error, urllib.error.HTTPError):
                self.addCleanup(error.close)
            with self.subTest(error=error):
                with patch.object(fetcher.urllib.request, "urlopen", side_effect=[error, io.BytesIO(b"font")]) as request:
                    with patch.object(fetcher.time, "sleep") as sleep:
                        self.assertEqual(fetcher.fetch(URL), b"font")
                        self.assertEqual(request.call_count, 2)
                        sleep.assert_called_once_with(1)

    def test_permanent_http_error_is_not_retried(self):
        error = urllib.error.HTTPError(URL, 404, "missing", {}, None)
        self.addCleanup(error.close)
        with patch.object(fetcher.urllib.request, "urlopen", side_effect=error) as request:
            with patch.object(fetcher.time, "sleep") as sleep:
                with self.assertRaises(urllib.error.HTTPError):
                    fetcher.fetch(URL)
                self.assertEqual(request.call_count, 1)
                sleep.assert_not_called()

    def test_transient_failure_stops_after_three_attempts(self):
        error = urllib.error.URLError("unavailable")
        with patch.object(fetcher.urllib.request, "urlopen", side_effect=error) as request:
            with patch.object(fetcher.time, "sleep") as sleep:
                with self.assertRaises(urllib.error.URLError):
                    fetcher.fetch(URL)
                self.assertEqual(request.call_count, 3)
                self.assertEqual([call.args[0] for call in sleep.call_args_list], [1, 2])

    def test_response_read_remains_bounded(self):
        class Response(io.BytesIO):
            def read(self, size=-1):
                self.size = size
                return super().read(size)

        response = Response(b"font")
        with patch.object(fetcher.urllib.request, "urlopen", return_value=response):
            self.assertEqual(fetcher.fetch(URL), b"font")
        self.assertEqual(response.size, 16 * 1024 * 1024 + 1)


if __name__ == "__main__":
    unittest.main()
