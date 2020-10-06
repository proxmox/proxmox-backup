Ext.define('PBS.TaskSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsTaskSummary',

    title: gettext('Task Summary'),

    controller: {
	xclass: 'Ext.app.ViewController',

	types: [
	    "backup",
	    "prune",
	    "garbage_collection",
	    "sync",
	    "verify",
	],

	titles: {
	    "backup": gettext('Backups'),
	    "prune": gettext('Prunes'),
	    "garbage_collection": gettext('Garbage collections'),
	    "sync": gettext('Syncs'),
	},

	render_icon: function(state, count) {
	    let cls = 'question';
	    let color = 'faded';
	    switch (state) {
		case "error":
		    cls = "times-circle";
		    color = "critical";
		    break;
		case "warning":
		    cls = "exclamation-circle";
		    color = "warning";
		    break;
		case "ok":
		    cls = "check-circle";
		    color = "good";
		    break;
		default: break;
	    }

	    if (count < 1) {
		color = "faded";
	    }
	    cls += " " + color;
	    return `<i class="fa fa-${cls}"></i>`;
	},

	render_count: function(value, md, record, rowindex, colindex) {
	    let me = this;
	    let icon = me.render_icon(me.states[colindex], value);
	    return `${icon} ${value}`;
	},
    },

    updateTasks: function(source) {
	let me = this;
	let controller = me.getController();
	let data = [];
	controller.types.forEach((type) => {
	    source[type].type = controller.titles[type];
	    data.push(source[type]);
	});
	me.lookup('grid').getStore().setData(data);
    },

    layout: 'fit',
    bodyPadding: 15,
    minHeight: 166,

    // we have to wrap the grid in a panel to get the padding right
    items: [
	{
	    xtype: 'grid',
	    reference: 'grid',
	    hideHeaders: true,
	    border: false,
	    bodyBorder: false,
	    rowLines: false,
	    viewConfig: {
		stripeRows: false,
		trackOver: false,
	    },
	    scrollable: false,
	    disableSelection: true,

	    store: {
		data: [],
	    },

	    columns: [
		{
		    dataIndex: 'type',
		    flex: 1,
		},
		{
		    dataIndex: 'error',
		    renderer: 'render_count',
		},
		{
		    dataIndex: 'warning',
		    renderer: 'render_count',
		},
		{
		    dataIndex: 'ok',
		    renderer: 'render_count',
		},
	    ],
	},
    ],

});
