Ext.define('PBS.widget.UsageChart', {
    extend: 'Ext.container.Container',
    alias: 'widget.pbsUsageChart',

    layout: {
	type: 'hbox',
	align: 'center',
    },

    items: [
	{
	    width: 80,
	    xtype: 'box',
	    itemId: 'title',
	    data: {
		title: '',
	    },
	    tpl: '<h3>{title}:</h3>',
	},
	{
	    flex: 1,
	    xtype: 'cartesian',
	    downloadServerUrl: '-',
	    height: '100%',
	    itemId: 'chart',
	    border: false,
	    axes: [
		{
		    type: 'numeric',
		    position: 'right',
		    hidden: false,
		    minimum: 0,
		    // TODO: make this configurable?!
		    maximum: 1,
		    renderer: (axis, label) => `${label*100}%`,
		},
		{
		    type: 'time',
		    position: 'bottom',
		    hidden: true,
		},
	    ],

	    store: {
		trackRemoved: false,
		data: {},
	    },

	    series: [{
		type: 'line',
		xField: 'time',
		yField: 'val',
		fill: 'true',
		colors: ['#cfcfcf'],
		tooltip: {
		    trackMouse: true,
		    renderer: function(tooltip, record, ctx) {
			if (!record || !record.data) return;
			let date = new Date(record.data.time);
			date = Ext.Date.format(date, 'c');
			let value = (100*record.data.val).toFixed(2);
			tooltip.setHtml(
			    `${value} %<br />
			    ${date}`,
			);
		    },
		},
		style: {
		    lineWidth: 1.5,
		    opacity: 0.60,
		},
		marker: {
		    opacity: 0,
		    scaling: 0.01,
		    fx: {
			duration: 200,
			easing: 'easeOut',
		    },
		},
		highlightCfg: {
		    opacity: 1,
		    scaling: 1.5,
		},
	    }],
	},
    ],

    setData: function(data) {
	let me = this;
	let chart = me.chart;
	chart.store.setData(data);
	chart.redraw();
    },

    // the renderer for the tooltip and last value, default just the value
    renderer: Ext.identityFn,

    setTitle: function(title) {
	let me = this;
	me.title = title;
	me.getComponent('title').update({ title: title });
    },

    checkThemeColors: function() {
	let me = this;
	let rootStyle = getComputedStyle(document.documentElement);

	// get color
	let background = rootStyle.getPropertyValue("--pwt-panel-background").trim() || "#ffffff";
	let text = rootStyle.getPropertyValue("--pwt-text-color").trim() || "#000000";
	let gridStroke = rootStyle.getPropertyValue("--pwt-chart-grid-stroke").trim() || "#dddddd";

	// set the colors
	me.chart.setBackground(background);
	if (!me.color) {
	    me.chart.axes.forEach((axis) => {
		axis.setLabel({ color: text });
		axis.setTitle({ color: text });
		axis.setStyle({ strokeStyle: gridStroke });
	    });
	}
	me.chart.redraw();
    },

    initComponent: function() {
	var me = this;
	me.callParent();

	if (me.title) {
	    me.getComponent('title').update({ title: me.title });
	}
	me.chart = me.getComponent('chart');
	me.chart.timeaxis = me.chart.getAxes()[1];
	if (me.color) {
	    me.chart.series[0].setStyle({
		fill: me.color,
		stroke: me.color,
	    });
	}

	me.checkThemeColors();

	// switch colors on media query changes
	me.mediaQueryList = window.matchMedia("(prefers-color-scheme: dark)");
	me.themeListener = (e) => { me.checkThemeColors(); };
	me.mediaQueryList.addEventListener("change", me.themeListener);
    },
});
