package render

import "testing"

func TestTruncateLines(t *testing.T) {
	tests := []struct {
		name     string
		input    string
		maxLines int
		want     string
	}{
		{
			name:     "empty input returns placeholder",
			input:    "",
			maxLines: 5,
			want:     "(no output)",
		},
		{
			name:     "only newlines returns placeholder",
			input:    "\n\n\n",
			maxLines: 5,
			want:     "(no output)",
		},
		{
			name:     "fewer lines than max is unchanged",
			input:    "one\ntwo",
			maxLines: 5,
			want:     "one\ntwo",
		},
		{
			name:     "exactly max lines is unchanged",
			input:    "1\n2\n3\n4\n5",
			maxLines: 5,
			want:     "1\n2\n3\n4\n5",
		},
		{
			name:     "trailing newlines are trimmed",
			input:    "one\ntwo\n\n\n",
			maxLines: 5,
			want:     "one\ntwo",
		},
		{
			name:     "long input keeps last max lines with indicator",
			input:    "1\n2\n3\n4\n5\n6\n7",
			maxLines: 5,
			want:     "… (2 more lines)\n3\n4\n5\n6\n7",
		},
		{
			name:     "long input with trailing newline",
			input:    "1\n2\n3\n4\n5\n6\n",
			maxLines: 5,
			want:     "… (1 more lines)\n2\n3\n4\n5\n6",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := truncateLines(tt.input, tt.maxLines)
			if got != tt.want {
				t.Errorf("truncateLines(%q, %d) = %q, want %q", tt.input, tt.maxLines, got, tt.want)
			}
		})
	}
}
