Ext.define('PBS.TaskSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsTaskSummary',

    title: gettext('Task Summary (last Month)'),

    controller: {
	xclass: 'Ext.app.ViewController',

	render_count: function(value, md, record, rowindex, colindex) {
	    let cls = 'question';
	    let color = 'faded';
	    switch (colindex) {
		case 1:
		    cls = "times-circle";
		    color = "critical";
		    break;
		case 2:
		    cls = "exclamation-circle";
		    color = "warning";
		    break;
		case 3:
		    cls = "check-circle";
		    color = "good";
		    break;
		default: break;
	    }

	    if (value < 1) {
		color = "faded";
	    }
	    cls += " " + color;
	    return `<i class="fa fa-${cls}"></i> ${value}`;
	},
    },

    updateTasks: function(data) {
	let me = this;
	data.backup.type = gettext('Backups');
	data.prune.type = gettext('Prunes');
	data.garbage_collection.type = gettext('Garbage collections');
	data.sync.type = gettext('Syncs');
	me.lookup('grid').getStore().setData([
	    data.backup,
	    data.prune,
	    data.garbage_collection,
	    data.sync,
	]);
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
		data: []
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
	}
    ],

});
