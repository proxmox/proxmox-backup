// Summary Panel for a single datastore in overview
Ext.define('PBS.datastore.DataStoreListSummary', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDataStoreListSummary',
    mixins: ['Proxmox.Mixin.CBind'],

    cbind: {
	title: '{datastore}',
    },
    bodyPadding: 10,

    controller: {
	xclass: 'Ext.app.ViewController',
    },
    viewModel: {
	data: {
	    full: "N/A",
	    history: [],
	},

	stores: {
	    historystore: {
		data: [],
	    },
	},
    },
    setTasks: function(taskdata, since) {
	let me = this;
	me.down('pbsTaskSummary').updateTasks(taskdata, since);
    },

    setStatus: function(statusData) {
	let me = this;
	let vm = me.getViewModel();

	let usage = statusData.used/statusData.total;
	let usagetext = Ext.String.format(gettext('{0} of {1}'),
	    Proxmox.Utils.format_size(statusData.used),
	    Proxmox.Utils.format_size(statusData.total),
	);

	let usagePanel = me.lookup('usage');
	usagePanel.updateValue(usage, usagetext);

	let estimate = PBS.Utils.render_estimate(statusData['estimated-full-date']);
	vm.set('full', estimate);
	let last = 0;
	let data = statusData.history.map((val) => {
	    if (val === null) {
		val = last;
	    } else {
		last = val;
	    }
	    return val;
	});
	let historyStore = vm.getStore('historystore');
	historyStore.setData([
	    {
		history: data,
	    },
	]);
    },

    items: [
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },

	    defaults: {
		padding: 5,
	    },

	    items: [
		{
		    xtype: 'panel',
		    border: false,
		    flex: 1,
		},
		{
		    xtype: 'pmxInfoWidget',
		    iconCls: 'fa fa-fw fa-line-chart',
		    title: gettext('Estimated Full'),
		    width: 230,
		    printBar: false,
		    bind: {
			data: {
			    text: '{full}',
			},
		    },
		},
	    ],
	},
	{
	    // we cannot autosize a sparklineline widget,
	    // abuse a grid with a single column/row to do it for us
	    xtype: 'grid',
	    hideHeaders: true,
	    minHeight: 70,
	    border: false,
	    bodyBorder: false,
	    rowLines: false,
	    disableSelection: true,
	    viewConfig: {
		trackOver: false,
	    },
	    bind: {
		store: '{historystore}',
	    },
	    columns: [{
		xtype: 'widgetcolumn',
		flex: 1,
		dataIndex: 'history',
		widget: {
		    xtype: 'sparklineline',
		    bind: '{record.history}',
		    spotRadius: 0,
		    fillColor: '#ddd',
		    lineColor: '#555',
		    lineWidth: 0,
		    chartRangeMin: 0,
		    chartRangeMax: 1,
		    tipTpl: '{y:number("0.00")*100}%',
		    height: 60,
		},
	    }],
	},
	{
	    xtype: 'container',
	    layout: {
		type: 'hbox',
		align: 'stretch',
	    },

	    defaults: {
		padding: 5,
	    },

	    items: [
		{
		    xtype: 'proxmoxGauge',
		    warningThreshold: 0.8,
		    criticalThreshold: 0.95,
		    flex: 1,
		    reference: 'usage',
		},
		{
		    xtype: 'container',
		    flex: 2,
		    layout: {
			type: 'vbox',
			align: 'stretch',
		    },

		    defaults: {
			padding: 5,
		    },

		    items: [
			{
			    xtype: 'label',
			    text: gettext('Task Summary')
			        + ` (${Ext.String.format(gettext('{0} days'), 30)})`,
			},
			{
			    xtype: 'pbsTaskSummary',
			    border: false,
			    header: false,
			    subPanelModal: true,
			    flex: 2,
			    bodyPadding: 0,
			    minHeight: 0,
			    cbind: {
				datastore: '{datastore}',
			    },
			},
		    ],
		},
	    ],
	},
    ],
});
