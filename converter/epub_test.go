package converter

import (
	"strings"
	"testing"
)

func TestNavXHTMLNestsEntriesByNavigationPath(t *testing.T) {
	chapters := []*Chapter{
		{Title: "大模型面试题"},
		{Title: "RAG 面试题介绍", NavigationPath: []string{"RAG", "RAG 面试题介绍"}},
		{Title: "1. 什么是 RAG？", NavigationPath: []string{"RAG", "1. 什么是 RAG？"}},
		{Title: "Function Calling", NavigationPath: []string{"Tools", "Function Calling"}},
	}

	nav := navXHTML("en", chapters)

	for _, want := range []string{
		`<li><a href="chapters/chapter-1.xhtml">大模型面试题</a></li>`,
		`<li><span>RAG</span>`,
		`<li><a href="chapters/chapter-2.xhtml">RAG 面试题介绍</a></li>`,
		`<li><a href="chapters/chapter-3.xhtml">1. 什么是 RAG？</a></li>`,
		`<li><span>Tools</span>`,
		`<li><a href="chapters/chapter-4.xhtml">Function Calling</a></li>`,
	} {
		if !strings.Contains(nav, want) {
			t.Fatalf("navXHTML missing %q:\n%s", want, nav)
		}
	}

	ragStart := strings.Index(nav, `<li><span>RAG</span>`)
	ragIntro := strings.Index(nav, `chapters/chapter-2.xhtml`)
	ragChapter := strings.Index(nav, `chapters/chapter-3.xhtml`)
	toolsStart := strings.Index(nav, `<li><span>Tools</span>`)
	if ragStart < 0 || ragIntro < ragStart || ragChapter < ragIntro || toolsStart < ragChapter {
		t.Fatalf("navXHTML did not preserve grouped chapter order:\n%s", nav)
	}
}
