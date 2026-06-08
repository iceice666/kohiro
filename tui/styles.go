package tui

import (
	"github.com/charmbracelet/bubbles/list"
	"github.com/charmbracelet/lipgloss"
)

const (
	colorPurple  = lipgloss.Color("#CBA6F7")
	colorBlue    = lipgloss.Color("#89B4FA")
	colorGreen   = lipgloss.Color("#A6E3A1")
	colorRed     = lipgloss.Color("#F38BA8")
	colorYellow  = lipgloss.Color("#F9E2AF")
	colorTeal    = lipgloss.Color("#94E2D5")
	colorPeach   = lipgloss.Color("#FAB387")
	colorText    = lipgloss.Color("#CDD6F4")
	colorSubtext = lipgloss.Color("#A6ADC8")
	colorSurface = lipgloss.Color("#313244")
	colorOverlay = lipgloss.Color("#6C7086")
)

var (
	styleTabActive   = lipgloss.NewStyle().Bold(true).Foreground(colorPurple).Underline(true).Padding(0, 1)
	styleTabInactive = lipgloss.NewStyle().Foreground(colorSubtext).Padding(0, 1)
	styleTabBar      = lipgloss.NewStyle().BorderStyle(lipgloss.NormalBorder()).BorderBottom(true).BorderForeground(colorOverlay)
	styleFooter      = lipgloss.NewStyle().Foreground(colorSubtext)
	styleBreadcrumb  = lipgloss.NewStyle().Bold(true).Foreground(colorBlue)
	styleBreadcrumbSep = lipgloss.NewStyle().Foreground(colorOverlay)
	styleDir         = lipgloss.NewStyle().Bold(true).Foreground(colorTeal)
	styleError       = lipgloss.NewStyle().Foreground(colorRed)
	styleModal       = lipgloss.NewStyle().Border(lipgloss.RoundedBorder()).Padding(1, 2).BorderForeground(colorPurple)
	styleToastError  = lipgloss.NewStyle().Bold(true).Foreground(colorRed)
	styleToastOK     = lipgloss.NewStyle().Bold(true).Foreground(colorGreen)
	styleHeader      = lipgloss.NewStyle().Bold(true).Foreground(colorPurple)

	styleTagPublic  = lipgloss.NewStyle().Foreground(colorGreen)
	styleTagPrivate = lipgloss.NewStyle().Foreground(colorYellow)

	styleCommitHash   = lipgloss.NewStyle().Foreground(colorPeach).Bold(true)
	styleCommitAuthor = lipgloss.NewStyle().Foreground(colorBlue)
	styleCommitDate   = lipgloss.NewStyle().Foreground(colorSubtext)

	styleSeparator = lipgloss.NewStyle().Foreground(colorOverlay)
	styleKey       = lipgloss.NewStyle().Foreground(colorPurple).Bold(true)
)

// newStyledDelegate returns a list delegate with purple selection highlight.
func newStyledDelegate() list.DefaultDelegate {
	d := list.NewDefaultDelegate()
	d.Styles.SelectedTitle = d.Styles.SelectedTitle.
		Foreground(colorPurple).
		BorderForeground(colorPurple)
	d.Styles.SelectedDesc = d.Styles.SelectedDesc.
		Foreground(colorSubtext).
		BorderForeground(colorPurple)
	return d
}
