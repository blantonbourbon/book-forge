package converter

import "testing"

func TestEscapeXMLTextStripsIllegalChars(t *testing.T) {
	input := "hello\x00world\x01\x08\tkeep\nme\r" + string(rune(0xB)) + "done"
	got := escapeXMLText(input)
	// Null and other C0 controls (except tab/LF/CR) must be stripped.
	if got != "helloworld\tkeep\nme\rdone" {
		t.Fatalf("escapeXMLText = %q, want stripped controls", got)
	}

	escaped := escapeXMLText(`a & b < c > d`)
	if escaped != "a &amp; b &lt; c &gt; d" {
		t.Fatalf("escapeXMLText specials = %q", escaped)
	}
}

func TestEscapeXMLAttrStripsIllegalChars(t *testing.T) {
	got := escapeXMLAttr("x\x00y\"z")
	if got != "xy&quot;z" {
		t.Fatalf("escapeXMLAttr = %q, want xy&quot;z", got)
	}
}
